use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crate::AppWindow;
use crate::startup::Startup;

use super::wgc;

pub struct ScreenCaptureSession {
    stop_tx: Option<Sender<()>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ScreenCaptureSession {
    pub fn start(target: &AppWindow, log_dir: &Path) -> io::Result<Self> {
        Self::start_with(target, log_dir, &Startup::new())
    }

    pub(crate) fn start_with(
        target: &AppWindow,
        log_dir: &Path,
        startup: &Startup,
    ) -> io::Result<Self> {
        startup.enter("screen capture encoder and metadata initialization")?;
        let hwnd = target.hwnd;
        let log_dir = log_dir.to_path_buf();
        let worker_startup = startup.clone();
        Self::start_worker(startup, move |stop_rx, ready_tx| {
            wgc::run_capture(hwnd, &log_dir, stop_rx, ready_tx, worker_startup)
        })
    }

    pub(crate) fn start_worker(
        startup: &Startup,
        run: impl FnOnce(mpsc::Receiver<()>, Sender<io::Result<()>>) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("screen-capture".into())
            .spawn(move || run(stop_rx, ready_tx))?;

        // Own the worker before waiting, so all failure paths request stop and
        // join it before returning (and before the log directory is removed).
        let session = Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        };
        if let Err(error) = startup.wait(&ready_rx).and_then(|()| startup.check()) {
            let message = format!("{}: {error}", startup.stage());
            let cleanup = session.stop();
            return Err(io::Error::new(
                error.kind(),
                match cleanup {
                    Ok(()) => message,
                    Err(cleanup) => format!("{message} | Capture rollback: {cleanup}"),
                },
            ));
        }
        Ok(session)
    }

    pub fn stop(mut self) -> io::Result<()> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> io::Result<()> {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }

        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| io::Error::other("screen capture thread panicked"))?
        } else {
            Ok(())
        }
    }
}

impl Drop for ScreenCaptureSession {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

pub fn start_screen_capture(
    target: &AppWindow,
    log_dir: &Path,
) -> io::Result<ScreenCaptureSession> {
    ScreenCaptureSession::start(target, log_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    #[test]
    fn start_waits_for_ready_and_stop_joins_the_worker() {
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        let session = ScreenCaptureSession::start_worker(&Startup::new(), move |stop, ready| {
            ready.send(Ok(())).unwrap();
            stop.recv().unwrap();
            flag.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(!stopped.load(Ordering::SeqCst));
        session.stop().unwrap();
        assert!(stopped.load(Ordering::SeqCst));
    }

    #[test]
    fn failure_disconnect_and_no_first_frame_all_stop_and_join() {
        for mode in 0..3 {
            let startup = Startup::with_timeout(Duration::from_millis(100));
            startup.enter("first captured and encoded frame").unwrap();
            let stopped = Arc::new(AtomicBool::new(false));
            let flag = stopped.clone();
            let result = ScreenCaptureSession::start_worker(&startup, move |stop, ready| {
                match mode {
                    0 => {
                        ready
                            .send(Err(io::Error::other("encoder WriteSample failed")))
                            .unwrap();
                    }
                    1 => drop(ready),
                    _ => {
                        let _keep_ready_open = ready;
                        stop.recv().unwrap();
                    }
                }
                // Failure/disconnect also must request stop before returning.
                if mode < 2 {
                    stop.recv().unwrap();
                }
                flag.store(true, Ordering::SeqCst);
                Ok(())
            });
            let error = result.err().expect("startup must fail");
            assert!(stopped.load(Ordering::SeqCst));
            match mode {
                0 => assert!(error.to_string().contains("WriteSample failed")),
                1 => assert!(error.to_string().contains("worker exited")),
                _ => assert_eq!(error.kind(), io::ErrorKind::TimedOut),
            }
        }
    }
}
