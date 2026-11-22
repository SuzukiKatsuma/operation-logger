use std::cell::RefCell;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{
    AppWindow, InputLoggingSession, ScreenCaptureSession, create_operation_log_directory,
    list_running_applications, load_or_create_local_participant_config,
};

use crate::startup::Startup;

use super::MainWindow;
use super::app_model::{AppIdentity, AppItem, build_items};

struct ActiveSession {
    input: InputLoggingSession,
    capture: ScreenCaptureSession,
    log_dir: PathBuf,
}

struct PendingStart {
    startup: Startup,
    task: JoinHandle<Result<ActiveSession, String>>,
}

#[derive(Default)]
struct StartFeedback {
    error: Option<String>,
}

impl StartFeedback {
    fn begin(&mut self) {
        self.error = None;
    }

    fn fail(&mut self, message: String, warn: impl FnOnce()) {
        if self.error.is_none() {
            warn();
        }
        self.error = Some(message);
    }

    fn can_accept_success(&self) -> bool {
        self.error.is_none()
    }

    fn accept<T>(&mut self, result: &Result<T, String>, warn: impl FnOnce()) -> bool {
        match result {
            Ok(_) => self.can_accept_success(),
            Err(error) => {
                self.fail(error.clone(), warn);
                false
            }
        }
    }
}

struct GuiState {
    apps: Vec<AppItem>,
    selected_index: Option<usize>,
    session: Option<ActiveSession>,
    pending_start: Option<PendingStart>,
    start_feedback: StartFeedback,
    last_log_dir: Option<PathBuf>,
    local_participant_id: String,
    status_text: String,
}

impl GuiState {
    fn new(local_participant_id: String) -> Self {
        Self {
            apps: Vec::new(),
            selected_index: None,
            session: None,
            pending_start: None,
            start_feedback: StartFeedback::default(),
            last_log_dir: None,
            local_participant_id,
            status_text: "Ready".to_string(),
        }
    }

    fn is_logging(&self) -> bool {
        self.session.is_some()
    }

    fn is_busy(&self) -> bool {
        self.is_logging() || self.pending_start.is_some()
    }

    fn selected_app(&self) -> Option<&AppItem> {
        self.selected_index.and_then(|idx| self.apps.get(idx))
    }

    fn stop_active_session(&mut self) -> io::Result<()> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };

        let ActiveSession {
            input,
            capture,
            log_dir,
        } = session;

        self.last_log_dir = Some(log_dir);
        let mut first_error = None;

        if let Err(error) = capture.stop() {
            first_error = Some(error);
        }
        if let Err(error) = input.stop() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

pub fn run() -> Result<(), Box<dyn Error>> {
    let local_participant_config = load_or_create_local_participant_config()?;
    let ui = MainWindow::new()?;
    let state = Rc::new(RefCell::new(GuiState::new(
        local_participant_config.local_participant_id,
    )));

    {
        let mut state_mut = state.borrow_mut();
        refresh_apps(&ui, &mut state_mut, true);
        sync_ui(&ui, &state_mut);
    }

    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        ui.on_refresh_clicked(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state_mut = state.borrow_mut();
                if state_mut.is_busy() {
                    return;
                }
                refresh_apps(&ui, &mut state_mut, false);
                sync_ui(&ui, &state_mut);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        ui.on_selection_changed(move |index| {
            if let Some(ui) = weak.upgrade() {
                let mut state_mut = state.borrow_mut();
                if state_mut.is_busy() {
                    return;
                }
                update_selection(&mut state_mut, index);
                sync_ui(&ui, &state_mut);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        ui.on_start_clicked(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state_mut = state.borrow_mut();
                start_logging(&mut state_mut);
                sync_ui(&ui, &state_mut);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        ui.on_stop_clicked(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state_mut = state.borrow_mut();
                stop_logging(&mut state_mut);
                sync_ui(&ui, &state_mut);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        ui.on_open_folder_clicked(move || {
            if let Some(ui) = weak.upgrade() {
                let mut state_mut = state.borrow_mut();
                open_log_folder(&mut state_mut);
                sync_ui(&ui, &state_mut);
            }
        });
    }

    // Poll only the startup task. All initialization and failure cleanup stays
    // off the event loop, including native calls that overrun the deadline.
    let startup_timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let state = Rc::clone(&state);
        startup_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                if let Some(ui) = weak.upgrade() {
                    let mut state = state.borrow_mut();
                    if state.pending_start.is_some() {
                        poll_start(&mut state, play_start_warning);
                        sync_ui(&ui, &state);
                    }
                }
            },
        );
    }

    let run_result = ui.run();
    startup_timer.stop();
    // Closing during startup revokes adoption. Retain the worker and wait for
    // its rollback instead of abandoning hooks or an encoder in the background.
    if let Some(pending) = state.borrow_mut().pending_start.take() {
        pending.startup.cancel();
        if let Ok(Ok(session)) = pending.task.join() {
            let _ = rollback_active_start(session);
        }
    }
    run_result?;

    let mut state_mut = state.borrow_mut();
    let _ = state_mut.stop_active_session();

    Ok(())
}

fn refresh_apps(ui: &MainWindow, state: &mut GuiState, is_initial: bool) {
    let preserve = state
        .selected_app()
        .map(|item| AppIdentity::from_app(&item.app));

    match list_running_applications() {
        Ok(apps) => {
            state.apps = build_items(apps);
            state.selected_index = preserve_selected_index(&state.apps, preserve);

            if state.apps.is_empty() {
                state.selected_index = None;
                state.status_text = "No running applications found.".to_string();
            } else if state.selected_index.is_none() {
                state.status_text = if is_initial {
                    format!("Loaded {} applications. Select one.", state.apps.len())
                } else {
                    format!(
                        "Refreshed {} applications. Selection was cleared.",
                        state.apps.len()
                    )
                };
            } else if is_initial {
                state.status_text = format!("Loaded {} applications.", state.apps.len());
            } else {
                state.status_text = format!("Refreshed {} applications.", state.apps.len());
            }
        }
        Err(error) => {
            state.status_text = format!("Failed to load applications: {error}");
        }
    }

    ui.set_app_items(model_from_apps(&state.apps));
}

fn preserve_selected_index(apps: &[AppItem], preserve: Option<AppIdentity>) -> Option<usize> {
    let target = preserve?;
    apps.iter().position(|item| target.matches(&item.app))
}

fn update_selection(state: &mut GuiState, index: i32) {
    if index < 0 {
        state.selected_index = None;
        state.status_text = "Selection cleared.".to_string();
        return;
    }

    let Ok(index) = usize::try_from(index) else {
        state.selected_index = None;
        state.status_text = "Invalid selection index.".to_string();
        return;
    };

    if index >= state.apps.len() {
        state.selected_index = None;
        state.status_text = "Selected item no longer exists.".to_string();
        return;
    }

    state.selected_index = Some(index);
    let app = &state.apps[index].app;
    state.status_text = format!(
        "Selected: {} (pid {}, {}).",
        app.title,
        app.process_id,
        app.process_name.as_deref().unwrap_or("unknown")
    );
}

fn start_logging(state: &mut GuiState) {
    if state.is_busy() {
        return;
    }
    state.start_feedback.begin();
    let Some(selected) = state.selected_app().map(|item| item.app.clone()) else {
        state
            .start_feedback
            .fail("Select an application first.".into(), play_start_warning);
        return;
    };

    let startup = Startup::new();
    let worker_startup = startup.clone();
    let participant = state.local_participant_id.clone();
    match thread::Builder::new()
        .name("recording-startup".into())
        .spawn(move || start_session(&selected, &participant, &worker_startup))
    {
        Ok(task) => {
            state.pending_start = Some(PendingStart { startup, task });
            state.status_text = "Starting... Waiting for all recorders to be ready.".into();
        }
        Err(error) => state.start_feedback.fail(
            format!("Failed to launch startup worker: {error}"),
            play_start_warning,
        ),
    }
}

fn start_session(
    selected: &AppWindow,
    participant: &str,
    startup: &Startup,
) -> Result<ActiveSession, String> {
    startup
        .enter("log directory initialization")
        .map_err(|e| e.to_string())?;
    let log_dir = create_operation_log_directory(selected, participant)
        .map_err(|error| format!("Failed to create log directory: {error}"))?;
    start_recorders(selected, &log_dir, startup)
        .map_err(|error| format_start_failure_status("Failed to start recording", error, &log_dir))
}

fn start_recorders(
    selected: &AppWindow,
    log_dir: &Path,
    startup: &Startup,
) -> io::Result<ActiveSession> {
    startup.check()?;
    let input = InputLoggingSession::start_with(selected, log_dir, startup)?;
    let capture = match ScreenCaptureSession::start_with(selected, log_dir, startup) {
        Ok(capture) => capture,
        Err(error) => {
            return Err(match input.stop() {
                Ok(()) => error,
                Err(cleanup) => io::Error::new(
                    error.kind(),
                    format!("{error} | Input rollback failed: {cleanup}"),
                ),
            });
        }
    };
    let session = ActiveSession {
        input,
        capture,
        log_dir: log_dir.to_path_buf(),
    };
    if let Err(error) = startup.check() {
        return Err(match stop_session(session) {
            Ok(()) => error,
            Err(cleanup) => io::Error::new(
                error.kind(),
                format!("{error} | Rollback failed: {cleanup}"),
            ),
        });
    }
    Ok(session)
}

fn poll_start(state: &mut GuiState, mut warn: impl FnMut()) {
    let Some(pending) = state.pending_start.as_ref() else {
        return;
    };
    if state.start_feedback.can_accept_success() {
        if let Err(error) = pending.startup.check() {
            pending.startup.cancel();
            state.start_feedback.fail(error.to_string(), &mut warn);
        }
    }
    if !pending.task.is_finished() {
        return;
    }
    let pending = state.pending_start.take().unwrap();
    let result = pending
        .task
        .join()
        .unwrap_or_else(|_| Err("Startup worker panicked.".into()));
    let accepted = state.start_feedback.accept(&result, &mut warn);
    match result {
        Ok(session) if accepted => {
            state.status_text = format!("Logging started: {}", session.log_dir.display());
            state.last_log_dir = Some(session.log_dir.clone());
            state.session = Some(session);
        }
        Ok(session) => {
            // A success arriving after timeout must never become a recording.
            // Keep Start disabled until its rollback has actually finished.
            let failure = state.start_feedback.error.clone().unwrap();
            state.pending_start = Some(PendingStart {
                startup: pending.startup,
                task: thread::spawn(move || {
                    Err(match rollback_active_start(session) {
                        Ok(()) => failure,
                        Err(error) => format!("{failure} | Rollback failed: {error}"),
                    })
                }),
            });
        }
        Err(_) => {}
    }
}

fn stop_session(session: ActiveSession) -> io::Result<()> {
    let capture_result = session.capture.stop();
    let input_result = session.input.stop();
    capture_result.and(input_result)
}

fn rollback_active_start(session: ActiveSession) -> io::Result<()> {
    let log_dir = session.log_dir.clone();
    let stop_result = stop_session(session);
    let cleanup_result = fs::remove_dir_all(log_dir);
    stop_result.and(cleanup_result)
}

fn play_start_warning() {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONWARNING;
    // SAFETY: MessageBeep has no pointer arguments; it queues the standard
    // Windows warning sound once and returns without waiting for playback.
    let _ = unsafe { MessageBeep(MB_ICONWARNING) };
}

fn format_start_failure_status(title: &str, start_error: io::Error, log_dir: &Path) -> String {
    match rollback_failed_start(log_dir) {
        Some(rollback_error) => {
            format!("{title}: {start_error} | Rollback failed: {rollback_error}")
        }
        None => format!("{title}: {start_error}"),
    }
}

fn rollback_failed_start(log_dir: &Path) -> Option<String> {
    let mut rollback_errors = Vec::new();

    if let Err(error) = fs::remove_dir_all(log_dir) {
        rollback_errors.push(format!("log dir cleanup error: {error}"));
    }

    if rollback_errors.is_empty() {
        None
    } else {
        Some(rollback_errors.join("; "))
    }
}

fn stop_logging(state: &mut GuiState) {
    if !state.is_logging() {
        state.status_text = "Logging is not running.".to_string();
        return;
    }

    match state.stop_active_session() {
        Ok(()) => {
            state.status_text = "Logging stopped.".to_string();
        }
        Err(error) => {
            state.status_text = format!("Failed to stop logging cleanly: {error}");
        }
    }
}

fn open_log_folder(state: &mut GuiState) {
    let Some(path) = state.last_log_dir.as_ref() else {
        state.status_text = "No log folder is available yet.".to_string();
        return;
    };

    match Command::new("explorer").arg(path).spawn() {
        Ok(_) => {
            state.status_text = format!("Opened log folder: {}", path.display());
        }
        Err(error) => {
            state.status_text = format!("Failed to open folder: {error}");
        }
    }
}

fn model_from_apps(apps: &[AppItem]) -> ModelRc<slint::SharedString> {
    let labels = apps
        .iter()
        .map(|item| item.label.clone())
        .collect::<Vec<_>>();
    ModelRc::new(VecModel::from(labels))
}

fn sync_ui(ui: &MainWindow, state: &GuiState) {
    ui.set_selected_index(
        state
            .selected_index
            .and_then(|idx| i32::try_from(idx).ok())
            .unwrap_or(-1),
    );

    if let Some(app) = state.selected_app().map(|item| &item.app) {
        ui.set_selected_title(app.title.clone().into());
        ui.set_selected_process_name(
            app.process_name
                .clone()
                .unwrap_or_else(|| "(unknown)".to_string())
                .into(),
        );
        ui.set_selected_process_id(app.process_id.to_string().into());
    } else {
        ui.set_selected_title("-".into());
        ui.set_selected_process_name("-".into());
        ui.set_selected_process_id("-".into());
    }

    ui.set_logging(state.is_logging());
    ui.set_starting(state.pending_start.is_some());
    ui.set_start_failed(state.start_feedback.error.is_some());
    ui.set_has_log_dir(state.last_log_dir.is_some());
    ui.set_local_participant_id(state.local_participant_id.clone().into());
    ui.set_status_text(
        state
            .start_feedback
            .error
            .as_ref()
            .map(|error| format!("START FAILED: {error}"))
            .unwrap_or_else(|| state.status_text.clone())
            .into(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path =
                PathBuf::from("target").join(format!("startup-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn wait_for_task(state: &GuiState) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !state.pending_start.as_ref().unwrap().task.is_finished() {
            assert!(Instant::now() < deadline, "startup task did not finish");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn successful_readiness_is_silent_and_failed_attempts_warn_once_until_retry() {
        let mut feedback = StartFeedback::default();
        let mut warnings = 0;
        feedback.begin();
        assert!(feedback.accept(&Ok::<_, String>(()), || warnings += 1));
        assert_eq!(warnings, 0);
        for reason in [
            "mouse hook",
            "keyboard hook",
            "controller Raw Input",
            "first frame acquisition",
            "first frame encoding",
        ] {
            feedback.begin();
            assert!(!feedback.accept(&Err::<(), _>(reason.into()), || warnings += 1));
            let after_first = warnings;
            feedback.fail(format!("{reason}: cleanup finished"), || warnings += 1);
            assert_eq!(warnings, after_first);
            // A late success cannot override an error from this attempt.
            assert!(!feedback.accept(&Ok::<_, String>(()), || warnings += 1));
            assert!(feedback.error.as_ref().unwrap().contains(reason));
        }
        feedback.begin();
        assert!(feedback.error.is_none());
        assert!(feedback.accept(&Ok::<_, String>(()), || warnings += 1));
        assert_eq!(warnings, 5);
    }

    #[test]
    fn initialization_failures_reach_error_state_and_rollback_all_started_recorders() {
        let target = AppWindow {
            hwnd: 0,
            title: "test".into(),
            process_id: 0,
            process_name: None,
        };
        // Run these serially: hooks/Raw Input have one sender per process.
        for (blocked, expected) in [
            (Some("mouse_input.csv"), "mouse CSV initialization"),
            (Some("mouse_move.csv"), "mouse CSV initialization"),
            (Some("keyboard_input.csv"), "keyboard CSV initialization"),
            (
                Some("controller_button_input.csv"),
                "controller CSV initialization",
            ),
            (
                Some("controller_analog_input.csv"),
                "controller CSV initialization",
            ),
            (Some("capture.mp4"), "MP4 encoder initialization"),
            (
                Some("capture_metadata.csv"),
                "capture metadata CSV initialization",
            ),
            (None, "Windows screen capture initialization"),
        ] {
            let directory = TestDirectory::new();
            if let Some(name) = blocked {
                fs::create_dir(directory.0.join(name)).unwrap();
            }
            let path = directory.0.clone();
            let selected = target.clone();
            let startup = Startup::new();
            let worker_startup = startup.clone();
            let mut state = GuiState::new("test".into());
            state.pending_start = Some(PendingStart {
                startup,
                task: thread::spawn(move || {
                    start_recorders(&selected, &path, &worker_startup).map_err(|e| {
                        format_start_failure_status("Failed to start recording", e, &path)
                    })
                }),
            });
            assert!(!state.is_logging());
            assert!(state.is_busy());
            wait_for_task(&state);
            let mut warnings = 0;
            poll_start(&mut state, || warnings += 1);
            assert_eq!(warnings, 1);
            assert!(!state.is_logging());
            assert!(!state.is_busy());
            let error = state.start_feedback.error.clone().unwrap();
            assert!(error.contains(expected), "expected {expected}: {error}");
            assert!(
                !directory.0.exists(),
                "failed start directory must be removed after workers stop"
            );
            update_selection(&mut state, -1);
            open_log_folder(&mut state);
            assert_eq!(state.start_feedback.error.as_ref(), Some(&error));
            poll_start(&mut state, || warnings += 1);
            assert_eq!(warnings, 1);
        }

        // A fresh real input session after rollback must be usable; CSV headers
        // must be visible before stop or the first input event.
        let directory = TestDirectory::new();
        let session = InputLoggingSession::start(&target, &directory.0).unwrap();
        for file in [
            "mouse_input.csv",
            "mouse_move.csv",
            "keyboard_input.csv",
            "controller_button_input.csv",
            "controller_analog_input.csv",
        ] {
            assert!(
                fs::read_to_string(directory.0.join(file))
                    .unwrap()
                    .starts_with("timestamp,")
            );
        }
        let capture = ready_capture();
        let mut state = GuiState::new("test".into());
        let path = directory.0.clone();
        state.pending_start = Some(PendingStart {
            startup: Startup::new(),
            task: thread::spawn(move || {
                Ok(ActiveSession {
                    input: session,
                    capture,
                    log_dir: path,
                })
            }),
        });
        let mut warnings = 0;
        wait_for_task(&state);
        poll_start(&mut state, || warnings += 1);
        assert!(state.is_logging());
        assert!(state.start_feedback.error.is_none());
        assert_eq!(warnings, 0);
        assert!(state.status_text.starts_with("Logging started:"));
        stop_logging(&mut state);

        // Even an otherwise successful set of recorders must be rolled back if
        // its result arrives after the attempt's deadline.
        let directory = TestDirectory::new();
        let input = InputLoggingSession::start(&target, &directory.0).unwrap();
        let capture = ready_capture();
        let path = directory.0.clone();
        state.pending_start = Some(PendingStart {
            startup: Startup::with_timeout(Duration::ZERO),
            task: thread::spawn(move || {
                Ok(ActiveSession {
                    input,
                    capture,
                    log_dir: path,
                })
            }),
        });
        wait_for_task(&state);
        poll_start(&mut state, || warnings += 1);
        assert!(!state.is_logging());
        assert!(state.is_busy());
        assert_eq!(warnings, 1);
        wait_for_task(&state);
        poll_start(&mut state, || warnings += 1);
        assert!(!state.is_logging());
        assert!(!state.is_busy());
        assert_eq!(warnings, 1);
        assert!(!directory.0.exists());
        assert!(
            state
                .start_feedback
                .error
                .as_ref()
                .unwrap()
                .contains("timed out")
        );
    }

    fn ready_capture() -> ScreenCaptureSession {
        ScreenCaptureSession::start_worker(&Startup::new(), |stop, ready| {
            ready.send(Ok(())).unwrap();
            stop.recv().unwrap();
            Ok(())
        })
        .unwrap()
    }

    #[test]
    fn timeout_keeps_gui_responsive_and_busy_until_cleanup_finishes() {
        let (release_tx, release_rx) = mpsc::channel();
        let mut state = GuiState::new("test".into());
        state.pending_start = Some(PendingStart {
            startup: Startup::with_timeout(Duration::ZERO),
            task: thread::spawn(move || {
                release_rx.recv().unwrap();
                Err("startup timed out; cleanup finished".into())
            }),
        });
        let mut warnings = 0;
        poll_start(&mut state, || warnings += 1);
        assert_eq!(warnings, 1);
        assert!(
            state
                .start_feedback
                .error
                .as_ref()
                .unwrap()
                .contains("timed out")
        );
        assert!(!state.is_logging());
        assert!(state.is_busy());
        // poll returned even though the background task is still blocked.
        poll_start(&mut state, || warnings += 1);
        assert_eq!(warnings, 1);
        release_tx.send(()).unwrap();
        wait_for_task(&state);
        poll_start(&mut state, || warnings += 1);
        assert_eq!(warnings, 1);
        assert!(!state.is_busy());
        assert!(!state.is_logging());
    }
}
