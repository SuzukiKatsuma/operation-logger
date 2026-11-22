use std::io;
use std::path::Path;

use crate::AppWindow;
use crate::startup::Startup;

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
        Self::start_with(target, log_dir, &Startup::new())
    }

    pub(crate) fn start_with(
        target: &AppWindow,
        log_dir: &Path,
        startup: &Startup,
    ) -> io::Result<Self> {
        let mut session = Self {
            controller: None,
            keyboard: None,
            mouse: None,
        };
        let result = (|| {
            session.mouse = Some(MouseLoggingSession::start(target, log_dir, startup)?);
            session.keyboard = Some(KeyboardLoggingSession::start(target, log_dir, startup)?);
            session.controller = Some(ControllerLoggingSession::start(target, log_dir, startup)?);
            startup.check()
        })();
        if let Err(error) = result {
            let message = format!("{}: {error}", startup.stage());
            return Err(io::Error::new(
                error.kind(),
                match session.stop_inner() {
                    Ok(()) => message,
                    Err(cleanup) => format!("{message} | Input rollback failed: {cleanup}"),
                },
            ));
        }
        Ok(session)
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
