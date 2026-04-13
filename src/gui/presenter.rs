use std::cell::RefCell;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{
    InputLoggingSession, ScreenCaptureSession, create_operation_log_directory,
    list_running_applications, start_input_logging, start_screen_capture,
};

use super::MainWindow;
use super::app_model::{AppIdentity, AppItem, build_items};

struct ActiveSession {
    input: InputLoggingSession,
    capture: ScreenCaptureSession,
    log_dir: PathBuf,
}

struct GuiState {
    apps: Vec<AppItem>,
    selected_index: Option<usize>,
    session: Option<ActiveSession>,
    last_log_dir: Option<PathBuf>,
    status_text: String,
}

impl GuiState {
    fn new() -> Self {
        Self {
            apps: Vec::new(),
            selected_index: None,
            session: None,
            last_log_dir: None,
            status_text: "Ready".to_string(),
        }
    }

    fn is_logging(&self) -> bool {
        self.session.is_some()
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
    let ui = MainWindow::new()?;
    let state = Rc::new(RefCell::new(GuiState::new()));

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

    ui.run()?;

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
    if state.is_logging() {
        state.status_text = "Logging is already running.".to_string();
        return;
    }

    let Some(selected) = state.selected_app().map(|item| item.app.clone()) else {
        state.status_text = "Select an application first.".to_string();
        return;
    };

    let log_dir = match create_operation_log_directory(&selected) {
        Ok(dir) => dir,
        Err(error) => {
            state.status_text = format!("Failed to create log directory: {error}");
            return;
        }
    };

    let input = match start_input_logging(&selected, &log_dir) {
        Ok(session) => session,
        Err(error) => {
            state.status_text = format!("Failed to start input logging: {error}");
            return;
        }
    };

    let capture = match start_screen_capture(&selected, &log_dir) {
        Ok(session) => session,
        Err(error) => {
            let _ = input.stop();
            state.status_text = format!("Failed to start screen capture: {error}");
            return;
        }
    };

    state.status_text = format!("Logging started: {}", log_dir.display());
    state.last_log_dir = Some(log_dir.clone());
    state.session = Some(ActiveSession {
        input,
        capture,
        log_dir,
    });
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
    ui.set_has_log_dir(state.last_log_dir.is_some());
    ui.set_status_text(state.status_text.clone().into());
}
