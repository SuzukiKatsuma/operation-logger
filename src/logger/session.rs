use std::io;
use std::path::Path;

use crate::AppWindow;

use super::controller::ControllerLoggingSession;
use super::keyboard::KeyboardLoggingSession;
use super::mouse::MouseLoggingSession;

pub struct InputLoggingSession {
    controller: Option<ControllerLoggingSession>,
    keyboard: Option<KeyboardLoggingSession>,
    mouse: Option<MouseLoggingSession>,
}

impl InputLoggingSession {
    pub fn start(target: &AppWindow, log_dir: &Path) -> io::Result<Self> {
        let mouse = MouseLoggingSession::start(target, log_dir)?;
        let keyboard = match KeyboardLoggingSession::start(target, log_dir) {
            Ok(keyboard) => keyboard,
            Err(error) => {
                let _ = mouse.stop();
                return Err(error);
            }
        };
        let controller = match ControllerLoggingSession::start(target, log_dir) {
            Ok(controller) => controller,
            Err(error) => {
                let _ = keyboard.stop();
                let _ = mouse.stop();
                return Err(error);
            }
        };

        Ok(Self {
            controller: Some(controller),
            keyboard: Some(keyboard),
            mouse: Some(mouse),
        })
    }

    pub fn stop(mut self) -> io::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<()> {
        let mut first_error = None;

        if let Some(controller) = self.controller.take() {
            if let Err(error) = controller.stop() {
                first_error = Some(error);
            }
        }

        if let Some(keyboard) = self.keyboard.take() {
            if let Err(error) = keyboard.stop() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        if let Some(mouse) = self.mouse.take() {
            if let Err(error) = mouse.stop() {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for InputLoggingSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

pub fn start_input_logging(target: &AppWindow, log_dir: &Path) -> io::Result<InputLoggingSession> {
    InputLoggingSession::start(target, log_dir)
}
