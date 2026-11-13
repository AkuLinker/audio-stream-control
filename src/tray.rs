//! Tray icon + right-click menu. Left-click opens the window.
//! Clicks only send messages; the PipeWire thread does the work.

use std::sync::mpsc;

use crate::{Cmd, Entry, HEADING, UiMsg, icon};

pub struct AppTray {
    pub entries: Vec<Entry>,
    pub status: Option<String>,
    tx: pipewire::channel::Sender<Cmd>,
    ui: mpsc::Sender<UiMsg>,
    icon_on: Vec<ksni::Icon>,
    icon_off: Vec<ksni::Icon>,
}

impl AppTray {
    pub fn new(tx: pipewire::channel::Sender<Cmd>, ui: mpsc::Sender<UiMsg>) -> Self {
        Self {
            entries: Vec::new(),
            status: None,
            tx,
            ui,
            icon_on: icon::tray_icons(true),
            icon_off: icon::tray_icons(false),
        }
    }

    fn sharing(&self) -> bool {
        self.entries.iter().any(|e| e.shared)
    }
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        match self.entries.iter().filter(|e| e.shared).count() {
            0 => "Audio sharing: off".into(),
            n => format!("Audio sharing: {n} source(s)"),
        }
    }

    // empty name → panels use our own pixmap below
    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.sharing() {
            self.icon_on.clone()
        } else {
            self.icon_off.clone()
        }
    }

    /// left click
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.ui.send(UiMsg::Open);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;

        let mut items: Vec<ksni::MenuItem<Self>> = vec![
            StandardItem {
                label: HEADING.into(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];

        if let Some(problem) = &self.status {
            items.push(
                StandardItem {
                    label: format!("⚠ {problem}"),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        if self.entries.is_empty() {
            items.push(
                StandardItem {
                    label: "(nothing is playing)".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
        }

        for e in &self.entries {
            let key = e.key.clone();
            items.push(
                CheckmarkItem {
                    label: label(e),
                    checked: e.shared,
                    activate: Box::new(move |this: &mut Self| {
                        let _ = this.tx.send(Cmd::Toggle(key.clone()));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Open window…".into(),
                icon_name: "window-new".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.ui.send(UiMsg::Open);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Stop sharing everything".into(),
                icon_name: "process-stop".into(),
                enabled: self.sharing(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.tx.send(Cmd::StopAll);
                }),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.ui.send(UiMsg::Quit);
                }),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

/// Text for one entry (shared by menu and window).
pub fn label(e: &Entry) -> String {
    if e.present {
        e.key.clone()
    } else {
        format!("{} (waiting)", e.key)
    }
}
