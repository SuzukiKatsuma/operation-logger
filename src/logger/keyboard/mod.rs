use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::AppWindow;

mod csv_writer;
pub(super) mod foreground;
mod hook;
mod model;

use self::csv_writer::KeyboardCsvWriter;
use self::foreground::ForegroundResolver;
use self::hook::KeyboardHook;
use self::model::{KeyboardInputEvent, KeyboardInputKind, KeyboardKeyId, RawKeyboardEvent};

pub(super) struct KeyboardLoggingSession {
    hook: Option<KeyboardHook>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl KeyboardLoggingSession {
    pub(super) fn start(selected: &AppWindow, log_dir: &Path) -> io::Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();
        let foreground = ForegroundResolver::new(selected);
        let log_dir = PathBuf::from(log_dir);

        let worker = thread::spawn(move || {
            let mut writer = KeyboardCsvWriter::new(&log_dir)?;
            let mut filter = KeyboardEventFilter::new();

            for event in event_rx {
                if !foreground.is_target_foreground() {
                    filter.clear();
                    continue;
                }

                let Some(event) = filter.accept(event) else {
                    continue;
                };

                writer.write_input(&event)?;
            }

            writer.flush()
        });

        let hook = match KeyboardHook::start(event_tx) {
            Ok(hook) => hook,
            Err(error) => {
                let _ = worker.join();
                return Err(error);
            }
        };

        Ok(Self {
            hook: Some(hook),
            worker: Some(worker),
        })
    }

    pub(super) fn stop(mut self) -> io::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<()> {
        if let Some(hook) = self.hook.take() {
            hook.stop()?;
        }

        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("keyboard logging worker panicked"))?
        } else {
            Ok(())
        }
    }
}

impl Drop for KeyboardLoggingSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

#[derive(Debug, Default)]
struct KeyboardEventFilter {
    pressed: HashSet<KeyboardKeyId>,
}

impl KeyboardEventFilter {
    fn new() -> Self {
        Self::default()
    }

    fn accept(&mut self, event: RawKeyboardEvent) -> Option<KeyboardInputEvent> {
        match event.kind {
            KeyboardInputKind::Down => {
                if !self.pressed.insert(event.key) {
                    return None;
                }
            }
            KeyboardInputKind::Up => {
                if !self.pressed.remove(&event.key) {
                    return None;
                }
            }
        }

        Some(KeyboardInputEvent::from_raw(event))
    }

    fn clear(&mut self) {
        self.pressed.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::model::{KeyboardInputKind, KeyboardKeyId, RawKeyboardEvent};
    use super::*;

    #[test]
    fn suppresses_repeated_keydown_until_keyup() {
        let key = KeyboardKeyId {
            virtual_key: 0x41,
            scan_code: 0x1E,
            is_extended: false,
        };
        let mut filter = KeyboardEventFilter::new();

        assert!(filter.accept(raw(key, KeyboardInputKind::Down)).is_some());
        assert!(filter.accept(raw(key, KeyboardInputKind::Down)).is_none());
        assert!(filter.accept(raw(key, KeyboardInputKind::Up)).is_some());
        assert!(filter.accept(raw(key, KeyboardInputKind::Down)).is_some());
    }

    #[test]
    fn ignores_keyup_after_pressed_state_is_cleared() {
        let key = KeyboardKeyId {
            virtual_key: 0x41,
            scan_code: 0x1E,
            is_extended: false,
        };
        let mut filter = KeyboardEventFilter::new();

        assert!(filter.accept(raw(key, KeyboardInputKind::Down)).is_some());
        filter.clear();
        assert!(filter.accept(raw(key, KeyboardInputKind::Up)).is_none());
    }

    fn raw(key: KeyboardKeyId, kind: KeyboardInputKind) -> RawKeyboardEvent {
        RawKeyboardEvent {
            key,
            kind,
            is_injected: false,
        }
    }
}
