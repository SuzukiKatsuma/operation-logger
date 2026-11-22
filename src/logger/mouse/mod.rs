use std::io;
use std::path::Path;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::AppWindow;
use crate::startup::Startup;

mod csv_writer;
mod hook;
mod model;
mod target_resolver;

use self::csv_writer::MouseCsvWriter;
use self::hook::MouseHook;
use self::model::ResolvedMouseEvent;
use self::target_resolver::TargetResolver;

pub(super) struct MouseLoggingSession {
    hook: Option<MouseHook>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl MouseLoggingSession {
    pub(super) fn start(
        selected: &AppWindow,
        log_dir: &Path,
        startup: &Startup,
    ) -> io::Result<Self> {
        startup.enter("mouse CSV initialization")?;
        let mut writer = MouseCsvWriter::new(log_dir)?;
        startup.check()?;
        let (event_tx, event_rx) = mpsc::channel();
        let resolver = TargetResolver::new(selected);

        let worker = thread::spawn(move || {
            for event in event_rx {
                let Some(resolved) = resolver.resolve(event) else {
                    continue;
                };

                match resolved {
                    ResolvedMouseEvent::Move(event) => writer.write_move(&event)?,
                    ResolvedMouseEvent::Input(event) => writer.write_input(&event)?,
                }
            }

            writer.flush()
        });

        let hook = match MouseHook::start(event_tx, startup) {
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
        let stop_result = self.hook.take().map_or(Ok(()), |source| source.stop());

        let worker_result = if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("mouse logging worker panicked"))?
        } else {
            Ok(())
        };
        stop_result.and(worker_result)
    }
}

impl Drop for MouseLoggingSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}
