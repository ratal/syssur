//! Built-in project presets (Rover System, Doorbell System tutorial, Blank Project)
//! and cross-platform `.sysml` Import/Export workflows (native `rfd` file pickers on Desktop,
//! Blob download + paste/upload modal on Browser/WASM).

use crate::i18n::{tr, Locale, TextKey};
use egui::{RichText, ScrollArea, TextEdit};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectPreset {
    RoverSystem,
    DoorbellSystem,
    BlankProject,
}

impl ProjectPreset {
    pub fn files(self) -> (&'static str, Vec<(&'static str, &'static str)>) {
        match self {
            Self::RoverSystem => (
                "rover-system",
                vec![
                    (
                        "power.sysml",
                        include_str!("../../../examples/rover-system/power.sysml"),
                    ),
                    (
                        "chassis.sysml",
                        include_str!("../../../examples/rover-system/chassis.sysml"),
                    ),
                    (
                        "vehicle.sysml",
                        include_str!("../../../examples/rover-system/vehicle.sysml"),
                    ),
                    (
                        "views.sysml",
                        include_str!("../../../examples/rover-system/views.sysml"),
                    ),
                ],
            ),
            Self::DoorbellSystem => (
                "doorbell-system",
                vec![
                    (
                        "structure.sysml",
                        include_str!("../../../examples/doorbell-system/structure.sysml"),
                    ),
                    (
                        "behavior.sysml",
                        include_str!("../../../examples/doorbell-system/behavior.sysml"),
                    ),
                    (
                        "requirements.sysml",
                        include_str!("../../../examples/doorbell-system/requirements.sysml"),
                    ),
                ],
            ),
            Self::BlankProject => (
                "new-project",
                vec![(
                    "model.sysml",
                    "package Model {\n    import StandardViewDefinitions::*;\n\n    view Overview : GeneralView {\n        expose Model::**;\n    }\n}\n",
                )],
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IoModalKind {
    Import,
    Export,
}

#[derive(Clone, Debug)]
pub struct IoModalState {
    pub open: bool,
    pub kind: IoModalKind,
    pub paste_filename: String,
    pub paste_content: String,
    pub replace_workspace: bool,
}

impl Default for IoModalState {
    fn default() -> Self {
        Self {
            open: false,
            kind: IoModalKind::Import,
            paste_filename: "imported.sysml".to_string(),
            paste_content: String::new(),
            replace_workspace: true,
        }
    }
}

pub enum IoAction {
    ImportFiles {
        files: Vec<(String, String)>,
        replace_workspace: bool,
    },
    Status(String),
}

pub fn show_io_modal(
    ctx: &egui::Context,
    state: &mut IoModalState,
    active_file: &str,
    files: &BTreeMap<String, String>,
    locale: Locale,
) -> Option<IoAction> {
    if !state.open {
        return None;
    }
    let mut open = state.open;
    let mut result = None;

    egui::Window::new(tr(locale, TextKey::ImportExportTitle))
        .open(&mut open)
        .resizable(true)
        .default_width(480.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut state.kind,
                    IoModalKind::Import,
                    tr(locale, TextKey::ImportAction),
                );
                ui.selectable_value(
                    &mut state.kind,
                    IoModalKind::Export,
                    tr(locale, TextKey::ExportSysml),
                );
            });
            ui.separator();

            match state.kind {
                IoModalKind::Import => {
                    ui.checkbox(
                        &mut state.replace_workspace,
                        "Replace current project (open as new workspace)",
                    );
                    ui.separator();
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if ui.button(tr(locale, TextKey::ImportFromFile)).clicked() {
                            let picked = pick_sysml_files_native();
                            if !picked.is_empty() {
                                result = Some(IoAction::ImportFiles {
                                    files: picked,
                                    replace_workspace: state.replace_workspace,
                                });
                                state.open = false;
                            }
                        }
                        ui.separator();
                    }

                    ui.label(RichText::new(tr(locale, TextKey::ImportFromPaste)).strong());
                    ui.horizontal(|ui| {
                        ui.label(tr(locale, TextKey::FilenameLabel));
                        ui.text_edit_singleline(&mut state.paste_filename);
                    });
                    ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        ui.add_sized(
                            [ui.available_width(), 200.0],
                            TextEdit::multiline(&mut state.paste_content).code_editor(),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button(tr(locale, TextKey::ImportAction)).clicked() {
                            let name = state.paste_filename.trim();
                            if !name.is_empty() && !state.paste_content.trim().is_empty() {
                                let final_name = if name.ends_with(".sysml") {
                                    name.to_string()
                                } else {
                                    format!("{name}.sysml")
                                };
                                result = Some(IoAction::ImportFiles {
                                    files: vec![(final_name, state.paste_content.clone())],
                                    replace_workspace: state.replace_workspace,
                                });
                                state.open = false;
                            }
                        }
                        if ui.button(tr(locale, TextKey::Close)).clicked() {
                            state.open = false;
                        }
                    });
                }
                IoModalKind::Export => {
                    ui.label(format!("Active file: {active_file}"));
                    ui.horizontal(|ui| {
                        if ui.button(tr(locale, TextKey::ExportActiveFile)).clicked() {
                            if let Some(content) = files.get(active_file) {
                                result = Some(IoAction::Status(export_single_file(
                                    active_file,
                                    content,
                                )));
                            }
                        }
                        if ui.button(tr(locale, TextKey::ExportAllFiles)).clicked() {
                            result = Some(IoAction::Status(export_all_files(files)));
                        }
                    });

                    ui.separator();
                    if let Some(content) = files.get(active_file) {
                        ui.label(RichText::new(format!("Preview ({active_file}):")).weak());
                        let mut preview = content.clone();
                        ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            ui.add_sized(
                                [ui.available_width(), 200.0],
                                TextEdit::multiline(&mut preview).code_editor(),
                            );
                        });
                    }
                    if ui.button(tr(locale, TextKey::Close)).clicked() {
                        state.open = false;
                    }
                }
            }
        });

    state.open &= open;
    result
}

#[cfg(not(target_arch = "wasm32"))]
fn pick_sysml_files_native() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(paths) = rfd::FileDialog::new()
        .add_filter("SysML v2", &["sysml", "kerml"])
        .pick_files()
    {
        for path in paths {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("imported.sysml")
                    .to_string();
                out.push((name, text));
            }
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn export_single_file(name: &str, content: &str) -> String {
    if let Some(path) = rfd::FileDialog::new()
        .set_file_name(name)
        .add_filter("SysML v2", &["sysml"])
        .save_file()
    {
        match std::fs::write(&path, content) {
            Ok(()) => format!("Exported {}", path.display()),
            Err(err) => format!("Export error: {err}"),
        }
    } else {
        "Export cancelled".to_string()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn export_all_files(files: &BTreeMap<String, String>) -> String {
    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
        let mut count = 0usize;
        for (name, content) in files {
            if std::fs::write(dir.join(name), content).is_ok() {
                count += 1;
            }
        }
        format!("Exported {count} .sysml file(s) to {}", dir.display())
    } else {
        "Export cancelled".to_string()
    }
}

#[cfg(target_arch = "wasm32")]
fn export_single_file(name: &str, content: &str) -> String {
    match trigger_browser_download(name, content) {
        Ok(()) => format!("Downloaded {name}"),
        Err(err) => format!("Download error: {err}"),
    }
}

#[cfg(target_arch = "wasm32")]
fn export_all_files(files: &BTreeMap<String, String>) -> String {
    let mut count = 0usize;
    for (name, content) in files {
        if trigger_browser_download(name, content).is_ok() {
            count += 1;
        }
    }
    format!("Triggered download for {count} .sysml file(s)")
}

#[cfg(target_arch = "wasm32")]
fn trigger_browser_download(filename: &str, content: &str) -> Result<(), String> {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let parts = js_sys::Array::new();
    parts.push(&wasm_bindgen::JsValue::from_str(content));
    let opts = BlobPropertyBag::new();
    opts.set_type("text/plain;charset=utf-8");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &opts)
        .map_err(|e| format!("{e:?}"))?;
    let url = Url::create_object_url_with_blob(&blob).map_err(|e| format!("{e:?}"))?;
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let a: HtmlAnchorElement = document
        .create_element("a")
        .map_err(|e| format!("{e:?}"))?
        .dyn_into()
        .map_err(|_| "not anchor")?;
    a.set_href(&url);
    a.set_download(filename);
    a.click();
    let _ = Url::revoke_object_url(&url);
    Ok(())
}
