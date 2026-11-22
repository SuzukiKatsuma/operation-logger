//! A shared deadline for one start attempt. Native calls cannot be forcibly
//! interrupted; their owners retain resources until cooperative cleanup ends.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub(crate) struct Startup {
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    stage: Arc<Mutex<&'static str>>,
}

impl Startup {
    pub(crate) fn new() -> Self {
        Self::with_timeout(START_TIMEOUT)
    }

    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        Self {
            deadline: Instant::now() + timeout,
            cancelled: Arc::new(AtomicBool::new(false)),
            stage: Arc::new(Mutex::new("log directory initialization")),
        }
    }

    pub(crate) fn enter(&self, stage: &'static str) -> io::Result<()> {
        self.check()?;
        *self.stage.lock().unwrap() = stage;
        Ok(())
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        if Instant::now() >= self.deadline {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("startup timed out while waiting for {}", self.stage()),
            ))
        } else if self.cancelled.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "startup cancelled",
            ))
        } else {
            Ok(())
        }
    }

    pub(crate) fn stage(&self) -> &'static str {
        *self.stage.lock().unwrap()
    }

    pub(crate) fn wait<T>(&self, rx: &Receiver<io::Result<T>>) -> io::Result<T> {
        loop {
            self.check()?;
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(remaining.min(Duration::from_millis(25))) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other(format!(
                        "{} worker exited before startup completed",
                        self.stage()
                    )));
                }
            }
        }
    }

    pub(crate) fn message_thread(
        &self,
        rx: Receiver<io::Result<u32>>,
        thread: std::thread::JoinHandle<io::Result<()>>,
    ) -> io::Result<(u32, std::thread::JoinHandle<io::Result<()>>)> {
        match self.wait(&rx) {
            Ok(id) => Ok((id, thread)),
            Err(error) => {
                // Keep ownership even if a native initialization call runs past
                // the deadline. Quit its message loop as soon as it is ready.
                // This wait only runs on the startup worker, never the GUI.
                if let Ok(Ok(id)) = rx.recv() {
                    use windows::Win32::Foundation::{LPARAM, WPARAM};
                    use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
                    // SAFETY: readiness is sent after the message queue exists.
                    let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                }
                match thread.join() {
                    Ok(Err(worker_error))
                        if !matches!(
                            error.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                        ) =>
                    {
                        Err(worker_error)
                    }
                    Err(_) => Err(io::Error::new(
                        error.kind(),
                        format!("{error} | {} worker panicked", self.stage()),
                    )),
                    _ => Err(error),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn readiness_propagates_success_failure_disconnect_and_timeout() {
        let startup = Startup::new();
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(42)).unwrap();
        assert_eq!(startup.wait(&rx).unwrap(), 42);
        tx.send(Err(io::Error::other("writer initialization failed")))
            .unwrap();
        assert!(
            startup
                .wait(&rx)
                .unwrap_err()
                .to_string()
                .contains("writer initialization failed")
        );
        drop(tx);
        assert!(
            startup
                .wait(&rx)
                .unwrap_err()
                .to_string()
                .contains("worker exited")
        );

        let startup = Startup::with_timeout(Duration::from_millis(5));
        startup.enter("first captured and encoded frame").unwrap();
        let (_tx, rx) = mpsc::channel::<io::Result<()>>();
        let error = startup.wait(&rx).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            error
                .to_string()
                .contains("first captured and encoded frame")
        );
    }

    #[test]
    fn cancellation_is_shared() {
        let startup = Startup::new();
        startup.clone().cancel();
        assert_eq!(
            startup.check().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
    }

    #[test]
    fn message_thread_preserves_initialization_errors_before_readiness() {
        let startup = Startup::new();
        startup
            .enter("controller Raw Input initialization")
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            drop(tx);
            Err(io::Error::other("window class registration failed"))
        });
        let error = startup.message_thread(rx, thread).err().unwrap();
        assert_eq!(error.to_string(), "window class registration failed");
    }

    #[test]
    fn timed_out_message_thread_is_quit_and_joined_even_if_readiness_is_late() {
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetMessageW, MSG, PM_NOREMOVE, PeekMessageW,
        };

        let startup = Startup::with_timeout(Duration::ZERO);
        let (tx, rx) = mpsc::channel();
        let exited = Arc::new(AtomicBool::new(false));
        let flag = exited.clone();
        let thread = std::thread::spawn(move || {
            let mut message = MSG::default();
            // SAFETY: creates this thread's message queue using local storage.
            unsafe {
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            }
            // SAFETY: GetCurrentThreadId has no preconditions.
            tx.send(Ok(unsafe { GetCurrentThreadId() })).unwrap();
            // SAFETY: message points to valid local storage; WM_QUIT is posted
            // by message_thread when it receives the late readiness above.
            assert_eq!(unsafe { GetMessageW(&mut message, None, 0, 0) }.0, 0);
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        let error = startup.message_thread(rx, thread).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(exited.load(Ordering::SeqCst));
    }
}
