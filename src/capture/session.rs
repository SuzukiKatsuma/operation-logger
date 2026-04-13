use std::io;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};

use crate::AppWindow;

use super::wgc;

pub struct ScreenCaptureSession {
    stop_tx: Option<Sender<()>>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl ScreenCaptureSession {
    pub fn start(target: &AppWindow, log_dir: &Path) -> io::Result<Self> {
        let hwnd = target.hwnd;
        let log_dir = log_dir.to_path_buf();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();

        let thread = thread::spawn(move || wgc::run_capture(hwnd, &log_dir, stop_rx, ready_tx));

        ready_rx.recv().map_err(|_| {
            io::Error::other("screen capture thread exited before startup completed")
        })??;

        Ok(Self {
            stop_tx: Some(stop_tx),
            thread: Some(thread),
        })
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
