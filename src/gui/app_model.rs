use slint::SharedString;

use crate::AppWindow;

#[derive(Clone)]
pub struct AppItem {
    pub app: AppWindow,
    pub label: SharedString,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AppIdentity {
    hwnd: isize,
    process_id: u32,
    title: String,
}

impl AppIdentity {
    pub fn from_app(app: &AppWindow) -> Self {
        Self {
            hwnd: app.hwnd,
            process_id: app.process_id,
            title: app.title.clone(),
        }
    }

    pub fn matches(&self, app: &AppWindow) -> bool {
        self.hwnd == app.hwnd && self.process_id == app.process_id && self.title == app.title
    }
}

pub fn build_items(apps: Vec<AppWindow>) -> Vec<AppItem> {
    apps.into_iter()
        .map(|app| AppItem {
            label: build_label(&app).into(),
            app,
        })
        .collect()
}

fn build_label(app: &AppWindow) -> String {
    let process_name = app.process_name.as_deref().unwrap_or("(unknown)");
    format!(
        "{} | {} | pid:{}",
        app.title.trim(),
        process_name,
        app.process_id
    )
}
