use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::AppWindow;
use crate::startup::Startup;

mod csv_writer;
mod device_registry;
mod hid_mapper;
mod model;
mod raw_input;

use self::csv_writer::ControllerCsvWriter;
use self::device_registry::DeviceRegistry;
use self::hid_mapper::HidMapper;
use self::model::{ControllerState, RawControllerReport};
use self::raw_input::RawInputSession;
use super::keyboard::foreground::ForegroundResolver;

pub(super) struct ControllerLoggingSession {
    raw_input: Option<RawInputSession>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl ControllerLoggingSession {
    pub(super) fn start(
        selected: &AppWindow,
        log_dir: &Path,
        startup: &Startup,
    ) -> io::Result<Self> {
        startup.enter("controller CSV initialization")?;
        let mut writer = ControllerCsvWriter::new(log_dir)?;
        startup.check()?;
        let (event_tx, event_rx) = mpsc::channel::<RawControllerReport>();
        let foreground = ForegroundResolver::new(selected);

        let worker = thread::spawn(move || {
            let mut registry = DeviceRegistry::new();
            let mut mapper = HidMapper::new();
            let mut states = HashMap::new();

            for report in event_rx {
                if !foreground.is_target_foreground() {
                    states.clear();
                    continue;
                }

                let device_id = registry.device_id(report.device_handle);
                let Some(snapshot) = mapper.map_report(&report.report) else {
                    continue;
                };
                let state = states
                    .entry(device_id.clone())
                    .or_insert_with(ControllerState::new);
                let events = state.diff(&device_id, snapshot);

                for event in events.button_events {
                    writer.write_button(&event)?;
                }
                for event in events.analog_events {
                    writer.write_analog(&event)?;
                }
            }

            writer.flush()
        });

        let raw_input = match RawInputSession::start(event_tx, startup) {
            Ok(raw_input) => raw_input,
            Err(error) => {
                let _ = worker.join();
                return Err(error);
            }
        };

        Ok(Self {
            raw_input: Some(raw_input),
            worker: Some(worker),
        })
    }

    pub(super) fn stop(mut self) -> io::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<()> {
        let stop_result = self.raw_input.take().map_or(Ok(()), |source| source.stop());

        let worker_result = if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("controller logging worker panicked"))?
        } else {
            Ok(())
        };
        stop_result.and(worker_result)
    }
}

impl Drop for ControllerLoggingSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}
