//! `syssur-app`: Pure Rust `egui` + `wgpu` (`eframe` + `egui_dock` + `rstar` + `ewebsock`)
//! desktop and browser/WASM MBSE editor for SysML v2.
//!
//! Layout architecture: SysON-style minimal shell with collapsible, fixed-pixel resizable
//! Left Explorer (`255px`) and Right `[Details | .sysml Code]` Inspector (`340px`), framing
//! a central multi-tab `DockArea` (Diagrams + Traceability) that alone scales with the OS window.

mod canvas;
mod catalog;
mod i18n;
mod inspector;
mod io;

use canvas::{CanvasAction, DiagramCanvas, NodeMeta, PortEndpoint};
use catalog::{show_create_submenus, SysmlCatalogItem};
use egui::{Color32, FontFamily, FontId, Id, RichText, ScrollArea, TextEdit, TextStyle, Ui, WidgetText};
use egui_dock::{DockArea, DockState, Style, TabViewer};
use ewebsock::{WsEvent, WsMessage, WsReceiver, WsSender};
use i18n::{tr, Locale, TextKey};
use inspector::{show_inspector_panel, InspectorAction, InspectorDraft, InspectorTab, SelectedTarget};
use io::{show_io_modal, IoAction, IoModalKind, IoModalState, ProjectPreset};
use std::collections::{BTreeMap, BTreeSet};
use syssur_cmd::{apply_text_edit, format_sysml_name, plan_text_edit, CommandHistory, HistoryEntry, SyssurCommand};
use syssur_collab::{ProjectCollab, SyncMessage};
use syssur_lang::SysmlEngine;
use syssur_layout::{
    compute_layout_with_port_overrides, LayoutDirection, LayoutEdgeSpec, LayoutInput,
    LayoutNodeSpec, LayoutOutput, LayoutPortSpec, NodeShape, Point2, PortPlacement,
};
use syssur_model::{
    ActionNode, ActionNodeKind, DefinitionGraph, DiagnosticSeverity, ElementId, FeatureOrigin,
    InstanceNode, InstancePath, PackageNode, RelationshipKind, StateNode, SysmlElementKind,
    VerificationStatus, ViewFilter, ViewKind,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum CenterTab {
    View(usize),
    Traceability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WireMode {
    Connect,
    Interface,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ViewScope {
    SysmlView(ElementId),
    AdHocRoot(ElementId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnStatus {
    Offline,
    Connecting,
    Online,
}

struct WsClient {
    sender: WsSender,
    receiver: WsReceiver,
}

#[derive(Clone, Debug)]
struct AppSettings {
    ui_scale: f32,
    base_font_size: f32,
    auto_hidpi: bool,
    locale: Locale,
    settings_open: bool,
    initialized_dpi: bool,
    last_applied_scale: f32,
    last_applied_font: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ui_scale: 1.15,
            base_font_size: 14.5,
            auto_hidpi: true,
            locale: Locale::En,
            settings_open: false,
            initialized_dpi: false,
            last_applied_scale: 0.0,
            last_applied_font: 0.0,
        }
    }
}

impl AppSettings {
    fn effective_zoom(&self, native_ppp: f32) -> f32 {
        let hidpi_boost = if self.auto_hidpi && native_ppp >= 1.5 {
            1.1
        } else {
            1.0
        };
        (self.ui_scale * hidpi_boost).clamp(0.75, 2.75)
    }
}

#[derive(Clone, Debug)]
struct RenameDialog {
    old_name: String,
    file: String,
    new_name: String,
}

struct ViewTabState {
    title: String,
    scope: ViewScope,
    view_kind: ViewKind,
    direction: LayoutDirection,
    palette_open: bool,
    palette_search: String,
    active_tool: Option<&'static SysmlCatalogItem>,
    base_input: LayoutInput,
    node_offsets: BTreeMap<String, Point2>,
    port_overrides: BTreeMap<(String, String), PortPlacement>,
    layout: LayoutOutput,
    node_meta: BTreeMap<String, NodeMeta>,
    canvas: DiagramCanvas,
    needs_fit: bool,
}

impl ViewTabState {
    fn new(title: String, scope: ViewScope, view_kind: ViewKind) -> Self {
        let direction = default_direction_for_view(view_kind);
        Self {
            title,
            scope,
            view_kind,
            direction,
            palette_open: false,
            palette_search: String::new(),
            active_tool: None,
            base_input: LayoutInput::default(),
            node_offsets: BTreeMap::new(),
            port_overrides: BTreeMap::new(),
            layout: LayoutOutput::default(),
            node_meta: BTreeMap::new(),
            canvas: DiagramCanvas::default(),
            needs_fit: true,
        }
    }

    fn recompute_layout_only(&mut self) {
        self.layout = compute_layout_with_port_overrides(
            &self.base_input,
            &self.node_offsets,
            &self.port_overrides,
        );
        self.canvas.rebuild_spatial_index(&self.layout);
    }
}

fn default_direction_for_view(kind: ViewKind) -> LayoutDirection {
    match kind {
        ViewKind::General | ViewKind::ActionFlow | ViewKind::StateTransition => {
            LayoutDirection::TopToBottom
        }
        ViewKind::Interconnection => LayoutDirection::LeftToRight,
    }
}

pub struct SyssurApp {
    project_name: String,
    collab: ProjectCollab,
    active_file: String,
    active_root: ElementId,
    wire_mode: WireMode,
    graph: DefinitionGraph,
    explorer_open: bool,
    explorer_search: String,
    inspector_open: bool,
    inspector_tab: InspectorTab,
    selected_target: SelectedTarget,
    inspector_draft: InspectorDraft,
    view_tabs: BTreeMap<usize, ViewTabState>,
    next_tab_id: usize,
    active_view_tab: Option<usize>,
    center_dock: DockState<CenterTab>,
    history: CommandHistory,
    command_input: String,
    ws_url: String,
    ws_client: Option<WsClient>,
    conn_status: ConnStatus,
    auto_reconnect: bool,
    last_reconnect_attempt: f64,
    settings: AppSettings,
    io_modal: IoModalState,
    rename_dialog: Option<RenameDialog>,
    status_line: String,
}

impl Default for SyssurApp {
    fn default() -> Self {
        let mut app = Self {
            project_name: "rover-system".to_string(),
            collab: ProjectCollab::default(),
            active_file: "vehicle.sysml".to_string(),
            active_root: ElementId::new("RoverSystem::Rover"),
            wire_mode: WireMode::Connect,
            graph: DefinitionGraph::default(),
            explorer_open: true,
            explorer_search: String::new(),
            inspector_open: true,
            inspector_tab: InspectorTab::Details,
            selected_target: SelectedTarget::None,
            inspector_draft: InspectorDraft::default(),
            view_tabs: BTreeMap::new(),
            next_tab_id: 1,
            active_view_tab: None,
            center_dock: DockState::new(vec![CenterTab::Traceability]),
            history: CommandHistory::default(),
            command_input: String::new(),
            ws_url: "ws://127.0.0.1:8090/ws/projects/rover-system".to_string(),
            ws_client: None,
            conn_status: ConnStatus::Offline,
            auto_reconnect: false,
            last_reconnect_attempt: 0.0,
            settings: AppSettings::default(),
            io_modal: IoModalState::default(),
            rename_dialog: None,
            status_line: "Offline mode (local Loro CRDT)".to_string(),
        };
        app.load_preset(ProjectPreset::RoverSystem);
        app
    }
}

impl SyssurApp {
    fn load_preset(&mut self, preset: ProjectPreset) {
        let (proj_name, files) = preset.files();
        self.project_name = proj_name.to_string();
        self.collab = ProjectCollab::default();
        for (name, src) in &files {
            let _ = self.collab.open_file(*name, *src);
        }
        self.active_file = files
            .last()
            .map(|(n, _)| (*n).to_string())
            .unwrap_or_else(|| "model.sysml".to_string());
        if preset == ProjectPreset::RoverSystem {
            self.active_file = "vehicle.sysml".to_string();
        }
        self.history = CommandHistory::default();
        self.selected_target = SelectedTarget::None;
        self.inspector_draft.invalidate();
        self.view_tabs.clear();
        self.next_tab_id = 1;
        self.active_view_tab = None;

        let texts = self.collab.snapshot_texts();
        self.graph = SysmlEngine::parse_workspace(&texts).graph;
        self.active_root = self
            .graph
            .part_defs
            .keys()
            .next()
            .or_else(|| self.graph.top_level_parts.keys().next())
            .cloned()
            .unwrap_or_else(|| ElementId::new("Model"));

        let mut initial_tabs = Vec::new();
        let views: Vec<_> = self.graph.views.values().cloned().collect();
        for v in views.iter().take(2) {
            let tid = self.allocate_view_tab(
                v.name.clone(),
                ViewScope::SysmlView(v.id.clone()),
                v.kind,
            );
            initial_tabs.push(CenterTab::View(tid));
        }
        if initial_tabs.is_empty() {
            let tid = self.allocate_view_tab(
                self.active_root.short_name().to_string(),
                ViewScope::AdHocRoot(self.active_root.clone()),
                ViewKind::General,
            );
            initial_tabs.push(CenterTab::View(tid));
        }
        initial_tabs.push(CenterTab::Traceability);
        self.center_dock = DockState::new(initial_tabs);
        self.active_view_tab = self.view_tabs.keys().next().copied();
        self.reparse_and_relayout();
        self.status_line = format!("Loaded project '{proj_name}'");
    }

    fn allocate_view_tab(&mut self, title: String, scope: ViewScope, kind: ViewKind) -> usize {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        self.view_tabs
            .insert(id, ViewTabState::new(title, scope, kind));
        id
    }

    fn open_imported_files(&mut self, imported: Vec<(String, String)>, replace_workspace: bool) {
        if imported.is_empty() {
            return;
        }
        if replace_workspace {
            let stem = std::path::Path::new(&imported[0].0)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("imported")
                .to_string();
            self.project_name = stem;
            self.collab = ProjectCollab::default();
            self.history = CommandHistory::default();
            self.selected_target = SelectedTarget::None;
            self.inspector_draft.invalidate();
            self.view_tabs.clear();
            self.next_tab_id = 1;
            self.active_view_tab = None;
        }

        let mut imported_names = BTreeSet::new();
        for (name, content) in imported {
            let _ = self.collab.open_file(&name, &content);
            imported_names.insert(name.clone());
            self.active_file = name;
        }

        let texts = self.collab.snapshot_texts();
        self.graph = SysmlEngine::parse_workspace(&texts).graph;

        fn count_instance_nodes(node: &InstanceNode) -> usize {
            1 + node.children.iter().map(count_instance_nodes).sum::<usize>()
        }

        let best_part = self
            .graph
            .top_level_parts
            .iter()
            .filter(|(_, p)| imported_names.contains(&p.span.file))
            .map(|(id, p)| (id.clone(), true, &p.span))
            .chain(
                self.graph
                    .part_defs
                    .iter()
                    .filter(|(_, d)| imported_names.contains(&d.span.file))
                    .map(|(id, d)| (id.clone(), false, &d.span)),
            )
            .filter_map(|(id, is_usage, span)| {
                let tree = InstanceNode::expand_from_graph(&self.graph, &id, 3)?;
                let count = count_instance_nodes(&tree);
                Some((count, is_usage, std::cmp::Reverse(span.clone()), id))
            })
            .max();

        let mut new_tab_ids = Vec::new();
        let mut new_center_tabs = Vec::new();

        if let Some((count, _, _, best_id)) = best_part {
            if count >= 2 {
                self.active_root = best_id.clone();
                let tid = self.allocate_view_tab(
                    best_id.short_name().to_string(),
                    ViewScope::AdHocRoot(best_id),
                    ViewKind::General,
                );
                new_tab_ids.push(tid);
                new_center_tabs.push(CenterTab::View(tid));
            }
        }

        if let Some(view) = self
            .graph
            .views
            .values()
            .filter(|v| v.is_usage && imported_names.contains(&v.span.file))
            .find(|v| {
                let (input, _) = build_layout_input_for_view(
                    &self.graph,
                    &ViewScope::SysmlView(v.id.clone()),
                    v.kind,
                    default_direction_for_view(v.kind),
                );
                !input.nodes.is_empty()
            })
        {
            let tid = self.allocate_view_tab(
                view.name.clone(),
                ViewScope::SysmlView(view.id.clone()),
                view.kind,
            );
            new_tab_ids.push(tid);
            new_center_tabs.push(CenterTab::View(tid));
        }

        if new_center_tabs.is_empty() {
            if let Some(st) = self
                .graph
                .states
                .values()
                .find(|s| imported_names.contains(&s.span.file) && !s.children.is_empty())
            {
                let tid = self.allocate_view_tab(
                    st.name.clone(),
                    ViewScope::AdHocRoot(st.id.clone()),
                    ViewKind::StateTransition,
                );
                new_tab_ids.push(tid);
                new_center_tabs.push(CenterTab::View(tid));
            } else if let Some(act) = self
                .graph
                .actions
                .values()
                .find(|a| imported_names.contains(&a.span.file) && !a.children.is_empty())
            {
                let tid = self.allocate_view_tab(
                    act.name.clone(),
                    ViewScope::AdHocRoot(act.id.clone()),
                    ViewKind::ActionFlow,
                );
                new_tab_ids.push(tid);
                new_center_tabs.push(CenterTab::View(tid));
            } else {
                let root_pkg = self
                    .graph
                    .packages
                    .values()
                    .filter(|p| imported_names.contains(&p.span.file))
                    .find(|p| {
                        !self.graph.packages.keys().any(|other| {
                            other != &p.id
                                && p.id
                                    .as_str()
                                    .starts_with(&format!("{}::", other.as_str()))
                        })
                    })
                    .or_else(|| {
                        self.graph
                            .packages
                            .values()
                            .find(|p| imported_names.contains(&p.span.file))
                    });
                if let Some(pkg) = root_pkg {
                    self.active_root = pkg.id.clone();
                    let tid = self.allocate_view_tab(
                        pkg.name.clone(),
                        ViewScope::AdHocRoot(pkg.id.clone()),
                        ViewKind::General,
                    );
                    new_tab_ids.push(tid);
                    new_center_tabs.push(CenterTab::View(tid));
                }
            }
        }

        if let Some(&first_tid) = new_tab_ids.first() {
            self.active_view_tab = Some(first_tid);
        }

        if replace_workspace {
            let mut dock_tabs = new_center_tabs;
            dock_tabs.push(CenterTab::Traceability);
            self.center_dock = DockState::new(dock_tabs);
        } else {
            for ct in &new_center_tabs {
                self.center_dock.push_to_focused_leaf(*ct);
            }
            if let Some(&first_tid) = new_tab_ids.first() {
                self.focus_center_tab(CenterTab::View(first_tid));
            }
        }

        self.reparse_and_relayout();
        for tid in new_tab_ids {
            if let Some(tab) = self.view_tabs.get_mut(&tid) {
                tab.needs_fit = true;
            }
        }
    }

    fn open_or_focus_view(&mut self, view_id: &ElementId) {
        if let Some((&tab_id, _)) = self
            .view_tabs
            .iter()
            .find(|(_, t)| matches!(&t.scope, ViewScope::SysmlView(id) if id == view_id))
        {
            self.active_view_tab = Some(tab_id);
            self.focus_center_tab(CenterTab::View(tab_id));
            return;
        }
        let (title, kind) = self
            .graph
            .views
            .get(view_id)
            .map(|v| (v.name.clone(), v.kind))
            .unwrap_or_else(|| (view_id.short_name().to_string(), ViewKind::General));
        let tab_id = self.allocate_view_tab(title, ViewScope::SysmlView(view_id.clone()), kind);
        self.active_view_tab = Some(tab_id);
        self.center_dock.push_to_focused_leaf(CenterTab::View(tab_id));
        self.reparse_and_relayout();
    }

    fn open_or_focus_root(&mut self, root_id: &ElementId, kind: ViewKind) {
        self.active_root = root_id.clone();
        if let Some((&tab_id, tab)) = self.view_tabs.iter_mut().find(
            |(_, t)| matches!(&t.scope, ViewScope::AdHocRoot(id) if id == root_id),
        ) {
            tab.view_kind = kind;
            tab.direction = default_direction_for_view(kind);
            tab.needs_fit = true;
            self.active_view_tab = Some(tab_id);
            self.focus_center_tab(CenterTab::View(tab_id));
            self.reparse_and_relayout();
            return;
        }
        let tab_id = self.allocate_view_tab(
            root_id.short_name().to_string(),
            ViewScope::AdHocRoot(root_id.clone()),
            kind,
        );
        self.active_view_tab = Some(tab_id);
        self.center_dock.push_to_focused_leaf(CenterTab::View(tab_id));
        self.reparse_and_relayout();
    }

    fn focus_center_tab(&mut self, target: CenterTab) {
        if let Some(tab_path) = self.center_dock.find_tab(&target) {
            let _ = self.center_dock.set_active_tab(tab_path);
        }
    }

    fn connect_ws(&mut self, now: f64) {
        self.last_reconnect_attempt = now;
        match ewebsock::connect(&self.ws_url, ewebsock::Options::default()) {
            Ok((sender, receiver)) => {
                self.ws_client = Some(WsClient { sender, receiver });
                self.conn_status = ConnStatus::Connecting;
                self.auto_reconnect = true;
                self.status_line = format!("Connecting to {}...", self.ws_url);
            }
            Err(err) => {
                self.ws_client = None;
                self.conn_status = ConnStatus::Offline;
                self.status_line = format!("WebSocket error: {err}");
            }
        }
    }

    fn disconnect_ws(&mut self) {
        self.ws_client = None;
        self.conn_status = ConnStatus::Offline;
        self.auto_reconnect = false;
        self.status_line = "Disconnected (offline edits buffered in Loro CRDT)".to_string();
    }

    fn send_sync_msg(&mut self, msg: &SyncMessage) {
        if let Some(client) = &mut self.ws_client {
            if self.conn_status == ConnStatus::Online {
                if let Ok(json) = serde_json::to_string(msg) {
                    client.sender.send(WsMessage::Text(json));
                }
            }
        }
    }

    fn poll_ws(&mut self, now: f64) {
        let mut events = Vec::new();
        if let Some(client) = &self.ws_client {
            while let Some(ev) = client.receiver.try_recv() {
                events.push(ev);
            }
        } else if self.auto_reconnect && (now - self.last_reconnect_attempt) > 3.0 {
            self.connect_ws(now);
            return;
        }

        let mut changed = false;
        let mut dropped = false;
        for ev in events {
            match ev {
                WsEvent::Opened => {
                    self.conn_status = ConnStatus::Online;
                    self.status_line = format!("Connected to {}", self.ws_url);
                    let hello = SyncMessage::Hello {
                        versions: self.collab.encode_versions(),
                    };
                    self.send_sync_msg(&hello);
                }
                WsEvent::Message(WsMessage::Text(txt)) => {
                    if let Ok(msg) = serde_json::from_str::<SyncMessage>(&txt) {
                        match msg {
                            SyncMessage::Hello { versions } => {
                                if let Ok(deltas) = self.collab.compute_missing_deltas(&versions) {
                                    let count = deltas.len();
                                    for (file, bytes) in deltas {
                                        self.send_sync_msg(&SyncMessage::Delta { file, bytes });
                                    }
                                    if count > 0 {
                                        self.status_line =
                                            format!("Reconciled {count} offline file delta(s)");
                                    }
                                }
                            }
                            SyncMessage::Delta { file, bytes } => {
                                if self.collab.import_file_delta(&file, &bytes).is_ok() {
                                    changed = true;
                                }
                            }
                            SyncMessage::CommitResult { commit_id, files } => {
                                self.status_line = format!(
                                    "Git snapshot committed: {commit_id} ({} .sysml files)",
                                    files.len()
                                );
                            }
                            SyncMessage::SnapshotRequest { .. }
                            | SyncMessage::CommitSnapshot { .. }
                            | SyncMessage::Presence(_) => {}
                        }
                    }
                }
                WsEvent::Closed | WsEvent::Error(_) => {
                    dropped = true;
                }
                WsEvent::Message(_) => {}
            }
        }

        if dropped {
            self.ws_client = None;
            self.conn_status = ConnStatus::Offline;
            self.last_reconnect_attempt = now;
            self.status_line = if self.auto_reconnect {
                "Connection lost — buffering locally, auto-reconnecting...".to_string()
            } else {
                "Disconnected".to_string()
            };
        }

        if changed {
            self.reparse_and_relayout();
        }
    }

    fn reparse_and_relayout(&mut self) {
        let texts = self.collab.snapshot_texts();
        let parsed = SysmlEngine::parse_workspace(&texts);
        self.graph = parsed.graph;

        if self.graph.resolve_part_def(&self.active_root).is_none()
            && !self.graph.top_level_parts.contains_key(&self.active_root)
        {
            if let Some(first) = self
                .graph
                .part_defs
                .keys()
                .next()
                .or_else(|| self.graph.top_level_parts.keys().next())
            {
                self.active_root = first.clone();
            }
        }

        // Remove open View tabs whose SysML view was deleted
        let stale_tabs: Vec<usize> = self
            .view_tabs
            .iter()
            .filter_map(|(&tid, tab)| match &tab.scope {
                ViewScope::SysmlView(vid) if !self.graph.views.contains_key(vid) => Some(tid),
                _ => None,
            })
            .collect();
        for tid in stale_tabs {
            self.view_tabs.remove(&tid);
            if let Some(tab_path) = self.center_dock.find_tab(&CenterTab::View(tid)) {
                self.center_dock.remove_tab(tab_path);
            }
        }

        let tab_ids: Vec<usize> = self.view_tabs.keys().copied().collect();
        for tab_id in tab_ids {
            if let Some(tab) = self.view_tabs.get(&tab_id) {
                let (input, meta) = build_layout_input_for_view(
                    &self.graph,
                    &tab.scope,
                    tab.view_kind,
                    tab.direction,
                );
                if let Some(tab_mut) = self.view_tabs.get_mut(&tab_id) {
                    tab_mut.base_input = input;
                    tab_mut.node_meta = meta;
                    tab_mut.recompute_layout_only();
                }
            }
        }
        self.inspector_draft.invalidate();
    }

    fn apply_and_broadcast_text(&mut self, file: &str, new_text: &str) {
        if let Ok(Some(delta)) = self.collab.set_file_text(file, new_text) {
            self.send_sync_msg(&SyncMessage::Delta {
                file: file.to_string(),
                bytes: delta,
            });
            self.reparse_and_relayout();
        }
    }

    fn execute_command(&mut self, cmd: SyssurCommand) {
        let file = cmd.target_file().to_string();
        let Some(doc) = self.collab.files.get(&file) else {
            self.status_line = format!("Command error: file '{file}' not open");
            return;
        };
        let before = doc.text();
        match plan_text_edit(&cmd, &before) {
            Ok(edit) => {
                let after = apply_text_edit(&before, &edit);
                self.history.record(HistoryEntry {
                    summary: cmd.summary(),
                    file: file.clone(),
                    before,
                    after: after.clone(),
                });
                self.status_line = format!("Executed: {}", cmd.summary());
                self.apply_and_broadcast_text(&file, &after);
            }
            Err(err) => {
                self.status_line = format!("Command failed: {err}");
            }
        }
    }

    fn connect_ports(&mut self, source: PortEndpoint, target: PortEndpoint) {
        let owner_id = source
            .node_path
            .parent()
            .and_then(|p| {
                target
                    .node_path
                    .parent()
                    .filter(|tp| *tp == p)
                    .and_then(|_| p.leaf().cloned())
            })
            .unwrap_or_else(|| self.active_root.clone());

        let (owner_def, file) = self
            .graph
            .resolve_part_def(&owner_id)
            .map(|pd| (pd.name.clone(), pd.span.file.clone()))
            .unwrap_or_else(|| (owner_id.short_name().to_string(), self.active_file.clone()));

        let src_ref = format!("{}.{}", source.node_label, source.port_name);
        let dst_ref = format!("{}.{}", target.node_label, target.port_name);

        let cmd = match self.wire_mode {
            WireMode::Connect => SyssurCommand::CreateConnection {
                file,
                owner_def,
                source: src_ref,
                target: dst_ref,
            },
            WireMode::Interface => {
                let idx = self.history.entries().len() + 1;
                SyssurCommand::CreateInterface {
                    file,
                    owner_def,
                    name: format!("link{idx}"),
                    interface_def: "PowerInterface".to_string(),
                    source: src_ref,
                    target: dst_ref,
                }
            }
        };
        self.execute_command(cmd);
    }

    fn undo(&mut self) {
        if let Some(entry) = self.history.undo() {
            self.status_line = format!("Undo: {}", entry.summary);
            self.apply_and_broadcast_text(&entry.file, &entry.before);
        }
    }

    fn redo(&mut self) {
        if let Some(entry) = self.history.redo() {
            self.status_line = format!("Redo: {}", entry.summary);
            self.apply_and_broadcast_text(&entry.file, &entry.after);
        }
    }

    fn default_package_for_file(&self, file: &str) -> String {
        self.graph
            .packages
            .values()
            .find(|p| p.span.file == file)
            .or_else(|| self.graph.packages.values().next())
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "Model".to_string())
    }

    fn default_action_for_file(&self, file: &str) -> Option<(String, String)> {
        self.graph
            .actions
            .values()
            .find(|a| a.span.file == file)
            .or_else(|| self.graph.actions.values().next())
            .map(|a| (a.name.clone(), a.span.file.clone()))
    }

    fn default_state_for_file(&self, file: &str) -> Option<(String, String)> {
        self.graph
            .states
            .values()
            .find(|s| s.span.file == file)
            .or_else(|| self.graph.states.values().next())
            .map(|s| (s.name.clone(), s.span.file.clone()))
    }

    fn place_catalog_item(
        &mut self,
        item: &'static SysmlCatalogItem,
        container_name: Option<String>,
        file: String,
    ) {
        let idx = self.history.entries().len() + 1;
        let name = match item.keyword {
            "start" | "done" | "terminate" | "import" | "filter" => item.default_name.to_string(),
            _ => format!("{}{idx}", item.default_name),
        };
        let pkg = container_name
            .clone()
            .unwrap_or_else(|| self.default_package_for_file(&file));

        if let Some(view_kind) = item.keyword.strip_prefix("view:") {
            self.execute_command(SyssurCommand::CreateView {
                file,
                package: pkg.clone(),
                name,
                view_kind: view_kind.to_string(),
                expose: vec![format!("{}::**", format_sysml_name(&pkg))],
            });
            return;
        }

        match item.keyword {
            "package" | "library package" => {
                self.execute_command(SyssurCommand::CreateSysmlElement {
                    file,
                    container: pkg,
                    keyword: item.keyword.to_string(),
                    name,
                    is_def: false,
                    specializes: None,
                    type_ref: None,
                    doc: None,
                });
            }
            "requirement" => {
                self.execute_command(SyssurCommand::CreateRequirement {
                    file,
                    package: pkg,
                    short_id: Some(format!("REQ-{idx:02}")),
                    name,
                    doc: item.doc_hint.to_string(),
                    is_usage: !item.is_def,
                });
            }
            "start" | "done" | "fork" | "join" | "decide" | "merge" => {
                let owner = container_name
                    .or_else(|| self.default_action_for_file(&file).map(|(n, _)| n));
                if let Some(act_owner) = owner {
                    self.execute_command(SyssurCommand::CreateControlNode {
                        file,
                        owner_action: act_owner,
                        kind: item.keyword.to_string(),
                        name,
                    });
                } else {
                    self.status_line = "Create an enclosing Action first".to_string();
                }
            }
            "port" if !item.is_def => {
                let owner = container_name
                    .unwrap_or_else(|| self.active_root.short_name().to_string());
                self.execute_command(SyssurCommand::CreatePort {
                    file,
                    owner_def: owner,
                    name,
                    port_def: "PowerPort".to_string(),
                    conjugated: false,
                });
            }
            "~port" => {
                let owner = container_name
                    .unwrap_or_else(|| self.active_root.short_name().to_string());
                self.execute_command(SyssurCommand::CreatePort {
                    file,
                    owner_def: owner,
                    name,
                    port_def: "PowerPort".to_string(),
                    conjugated: true,
                });
            }
            "part" if item.is_def => {
                self.execute_command(SyssurCommand::CreatePartDef {
                    file,
                    package: pkg,
                    name,
                    specializes: None,
                });
            }
            "part" => {
                self.execute_command(SyssurCommand::CreatePartUsage {
                    file,
                    owner_def: pkg,
                    name,
                    type_def: String::new(),
                });
            }
            "action" => {
                let target_container = container_name
                    .or_else(|| self.default_action_for_file(&file).map(|(n, _)| n))
                    .unwrap_or(pkg);
                self.execute_command(SyssurCommand::CreateAction {
                    file,
                    container: target_container,
                    name,
                    is_def: item.is_def,
                });
            }
            "state" if !item.is_def => {
                let target_container = container_name
                    .or_else(|| self.default_state_for_file(&file).map(|(n, _)| n))
                    .unwrap_or(pkg);
                self.execute_command(SyssurCommand::CreateSysmlElement {
                    file,
                    container: target_container,
                    keyword: "state".to_string(),
                    name,
                    is_def: false,
                    specializes: None,
                    type_ref: None,
                    doc: None,
                });
            }
            other => {
                self.execute_command(SyssurCommand::CreateSysmlElement {
                    file,
                    container: pkg,
                    keyword: other.to_string(),
                    name,
                    is_def: item.is_def,
                    specializes: None,
                    type_ref: None,
                    doc: if matches!(other, "comment" | "constraint" | "concern") {
                        Some(item.doc_hint.to_string())
                    } else {
                        None
                    },
                });
            }
        }
    }

    fn execute_relationship(
        &mut self,
        rel_keyword: &str,
        source_path: &InstancePath,
        source_label: &str,
        target_label: &str,
        guard: Option<String>,
    ) {
        let parent_short = source_path
            .parent()
            .and_then(|p| p.leaf().map(|l| l.short_name().to_string()));
        let (owner_scope, file) = match rel_keyword {
            "transition" => {
                let owner = parent_short
                    .or_else(|| self.default_state_for_file(&self.active_file).map(|(n, _)| n))
                    .unwrap_or_else(|| self.default_package_for_file(&self.active_file));
                let f = self
                    .graph
                    .resolve_state(&ElementId::new(&owner))
                    .map(|s| s.span.file.clone())
                    .unwrap_or_else(|| self.active_file.clone());
                (owner, f)
            }
            "succession" | "guarded_succession" => {
                let owner = parent_short
                    .or_else(|| self.default_action_for_file(&self.active_file).map(|(n, _)| n))
                    .unwrap_or_else(|| self.default_package_for_file(&self.active_file));
                let f = self
                    .graph
                    .resolve_action(&ElementId::new(&owner))
                    .map(|a| a.span.file.clone())
                    .unwrap_or_else(|| self.active_file.clone());
                (owner, f)
            }
            "dependency" | "satisfy" => {
                let f = source_path
                    .leaf()
                    .map(|id| self.file_for_element(id))
                    .unwrap_or_else(|| self.active_file.clone());
                let pkg = source_path
                    .0
                    .first()
                    .and_then(|id| id.as_str().split("::").next())
                    .map(str::to_string)
                    .unwrap_or_else(|| self.default_package_for_file(&f));
                (pkg, f)
            }
            "verify" => {
                let owner = source_label.to_string();
                let f = source_path
                    .leaf()
                    .map(|id| self.file_for_element(id))
                    .unwrap_or_else(|| self.active_file.clone());
                (owner, f)
            }
            _ => {
                let f = source_path
                    .leaf()
                    .map(|id| self.file_for_element(id))
                    .unwrap_or_else(|| self.active_file.clone());
                let owner = parent_short
                    .unwrap_or_else(|| self.default_package_for_file(&f));
                (owner, f)
            }
        };

        let effective_guard = if rel_keyword == "guarded_succession" && guard.is_none() {
            Some("true".to_string())
        } else {
            guard
        };

        self.execute_command(SyssurCommand::CreateRelationship {
            file,
            owner_scope,
            kind: rel_keyword.to_string(),
            source: source_label.to_string(),
            target: target_label.to_string(),
            label: None,
            guard: effective_guard,
        });
    }

    fn handle_canvas_action(&mut self, tab_id: usize, action: CanvasAction, ctx: &egui::Context) {
        match action {
            CanvasAction::ConnectPorts { source, target } => {
                self.connect_ports(source, target);
            }
            CanvasAction::IncompatibleWireDrop { reason } => {
                self.status_line = format!("Blocked wire: {reason}");
            }
            CanvasAction::ConnectNodes {
                source_path,
                source_label,
                target_label,
                kind,
                guard,
            } => {
                let kw = match kind {
                    RelationshipKind::Succession => "succession",
                    RelationshipKind::Transition => "transition",
                    RelationshipKind::Satisfy => "satisfy",
                    RelationshipKind::Verify => "verify",
                    RelationshipKind::Flow => "flow",
                    RelationshipKind::Allocate => "allocate",
                    RelationshipKind::Dependency => "dependency",
                    RelationshipKind::Binding => "binding",
                    RelationshipKind::Interface => "interface",
                    RelationshipKind::Specialization => "specialization",
                    _ => "connection",
                };
                self.execute_relationship(kw, &source_path, &source_label, &target_label, guard);
            }
            CanvasAction::CreateRelationship {
                rel_keyword,
                source_path,
                source_label,
                target_label,
            } => {
                self.execute_relationship(&rel_keyword, &source_path, &source_label, &target_label, None);
            }
            CanvasAction::PlaceCatalogItem {
                item,
                container,
                file,
            } => {
                let cname = container.and_then(|p| p.leaf().map(|l| l.short_name().to_string()));
                self.place_catalog_item(item, cname, file);
            }
            CanvasAction::MoveNodes { deltas } => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    for (key, dx, dy) in deltas {
                        let entry = tab
                            .node_offsets
                            .entry(key)
                            .or_insert(Point2 { x: 0.0, y: 0.0 });
                        entry.x += dx;
                        entry.y += dy;
                    }
                    tab.recompute_layout_only();
                }
            }
            CanvasAction::MovePort {
                node_key,
                port_name,
                placement,
            } => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    if let Some(pos) = placement {
                        tab.port_overrides.insert((node_key, port_name), pos);
                    } else {
                        tab.port_overrides.remove(&(node_key, port_name));
                    }
                    tab.recompute_layout_only();
                }
            }
            CanvasAction::ApplyCommand(cmd) => {
                self.execute_command(cmd);
            }
            CanvasAction::ExposeElement { target } => {
                if let Some(tab) = self.view_tabs.get(&tab_id) {
                    if let ViewScope::SysmlView(ref vid) = tab.scope {
                        if let Some(v) = self.graph.views.get(vid) {
                            let file = v.span.file.clone();
                            let view_name = v.name.clone();
                            let formatted_pattern = target
                                .as_str()
                                .split("::")
                                .map(format_sysml_name)
                                .collect::<Vec<_>>()
                                .join("::");
                            self.execute_command(SyssurCommand::ExposeInView {
                                file,
                                view_name,
                                target_pattern: formatted_pattern,
                            });
                            return;
                        }
                    }
                }
                self.status_line = "Drop onto a SysML View tab to expose".to_string();
            }
            CanvasAction::SelectNode { path } => {
                if let Some(leaf) = path.leaf().cloned() {
                    self.selected_target = SelectedTarget::Node {
                        path,
                        element_id: leaf,
                    };
                }
            }
            CanvasAction::SelectEdge { index } => {
                if let Some(tab) = self.view_tabs.get(&tab_id) {
                    if let Some(edge) = tab.layout.edges.get(index) {
                        if let Some(succ) = self
                            .graph
                            .successions
                            .iter()
                            .enumerate()
                            .find(|(i, s)| format!("{}#{i}", s.id) == edge.id)
                            .map(|(_, s)| s)
                        {
                            self.selected_target = SelectedTarget::Edge {
                                source: succ.source.clone(),
                                target: succ.target.clone(),
                                kind: RelationshipKind::Succession,
                                trigger: None,
                                guard: succ.guard.clone(),
                                effect: None,
                                file: succ.span.file.clone(),
                            };
                        } else if let Some(tr) = self
                            .graph
                            .transitions
                            .iter()
                            .enumerate()
                            .find(|(i, t)| format!("{}#{i}", t.id) == edge.id)
                            .map(|(_, t)| t)
                        {
                            self.selected_target = SelectedTarget::Edge {
                                source: tr.source.clone(),
                                target: tr.target.clone(),
                                kind: RelationshipKind::Transition,
                                trigger: tr.trigger.clone(),
                                guard: tr.guard.clone(),
                                effect: tr.effect.clone(),
                                file: tr.span.file.clone(),
                            };
                        } else {
                            self.selected_target = SelectedTarget::Edge {
                                source: edge.id.clone(),
                                target: edge.label.clone().unwrap_or_default(),
                                kind: edge.kind,
                                trigger: None,
                                guard: edge.guard.clone(),
                                effect: None,
                                file: self.active_file.clone(),
                            };
                        }
                    }
                }
            }
            CanvasAction::InspectAsRoot { target } => {
                self.open_or_focus_root(&target, ViewKind::Interconnection);
            }
            CanvasAction::RequestRename { element, file } => {
                let old_name = element.short_name().to_string();
                self.rename_dialog = Some(RenameDialog {
                    new_name: old_name.clone(),
                    old_name,
                    file,
                });
            }
            CanvasAction::DeleteElement { element, file } => {
                self.execute_command(SyssurCommand::DeleteElement {
                    file,
                    kind_hint: None,
                    name: element.short_name().to_string(),
                });
                self.selected_target = SelectedTarget::None;
            }
            CanvasAction::JumpToSource { file } => {
                if self.collab.files.contains_key(&file) {
                    self.active_file = file.clone();
                    self.inspector_open = true;
                    self.inspector_tab = InspectorTab::Code;
                    self.status_line = format!("Opened {file}");
                }
            }
            CanvasAction::CopyPath { path } => {
                ctx.copy_text(path.clone());
                self.status_line = format!("Copied path: {path}");
            }
            CanvasAction::AutoLayout => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    tab.node_offsets.clear();
                    tab.port_overrides.clear();
                    tab.recompute_layout_only();
                    tab.needs_fit = true;
                }
                self.status_line = "Reset to deterministic auto-layout".to_string();
            }
            CanvasAction::FitView => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    tab.needs_fit = true;
                }
            }
        }
    }

    fn apply_display_settings(&mut self, ctx: &egui::Context) {
        let native_ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
        if !self.settings.initialized_dpi {
            self.settings.initialized_dpi = true;
            if self.settings.auto_hidpi && native_ppp >= 1.5 {
                self.settings.ui_scale = 1.15;
                self.settings.base_font_size = 15.0;
            }
        }

        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Plus) {
                self.settings.ui_scale = (self.settings.ui_scale + 0.1).min(2.5);
            } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Minus) {
                self.settings.ui_scale = (self.settings.ui_scale - 0.1).max(0.8);
            } else if i.modifiers.ctrl && i.key_pressed(egui::Key::Num0) {
                self.settings.ui_scale = 1.15;
                self.settings.base_font_size = 14.5;
            }
        });

        let zoom = self.settings.effective_zoom(native_ppp);
        if (zoom - self.settings.last_applied_scale).abs() > 0.005 {
            ctx.set_zoom_factor(zoom);
            self.settings.last_applied_scale = zoom;
        }

        let base = self.settings.base_font_size;
        if (base - self.settings.last_applied_font).abs() > 0.1 {
            ctx.global_style_mut(|style| {
                style.text_styles.insert(
                    TextStyle::Small,
                    FontId::new((base - 2.0).max(9.0), FontFamily::Proportional),
                );
                style.text_styles.insert(
                    TextStyle::Body,
                    FontId::new(base, FontFamily::Proportional),
                );
                style.text_styles.insert(
                    TextStyle::Button,
                    FontId::new(base, FontFamily::Proportional),
                );
                style.text_styles.insert(
                    TextStyle::Monospace,
                    FontId::new((base - 0.5).max(10.0), FontFamily::Monospace),
                );
                style.text_styles.insert(
                    TextStyle::Heading,
                    FontId::new(base + 3.5, FontFamily::Proportional),
                );
            });
            self.settings.last_applied_font = base;
        }
    }

    fn handle_global_shortcuts(&mut self, ctx: &egui::Context) {
        let wants_kb = ctx.egui_wants_keyboard_input();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            for tab in self.view_tabs.values_mut() {
                tab.active_tool = None;
                tab.canvas.cancel_interactions();
            }
        }
        if !wants_kb {
            if ctx.input(|i| i.key_pressed(egui::Key::F2)) {
                self.open_rename_for_selected();
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
                self.delete_selected();
            }
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Z)) {
                self.undo();
            }
            if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::Y)) {
                self.redo();
            }
        }
    }

    fn file_for_element(&self, eid: &ElementId) -> String {
        self.graph
            .requirements
            .get(eid)
            .map(|r| r.span.file.clone())
            .or_else(|| self.graph.resolve_action(eid).map(|a| a.span.file.clone()))
            .or_else(|| self.graph.resolve_state(eid).map(|s| s.span.file.clone()))
            .or_else(|| self.graph.generic_elements.get(eid).map(|g| g.span.file.clone()))
            .or_else(|| self.graph.resolve_part_def(eid).map(|p| p.span.file.clone()))
            .or_else(|| self.graph.top_level_parts.get(eid).map(|p| p.span.file.clone()))
            .or_else(|| self.graph.packages.get(eid).map(|p| p.span.file.clone()))
            .or_else(|| self.graph.views.get(eid).map(|v| v.span.file.clone()))
            .unwrap_or_else(|| self.active_file.clone())
    }

    fn open_rename_for_selected(&mut self) {
        let target_id = match &self.selected_target {
            SelectedTarget::Node { element_id, .. } | SelectedTarget::Element(element_id) => {
                Some(element_id.clone())
            }
            _ => None,
        };
        let Some(eid) = target_id else {
            return;
        };
        let file = self.file_for_element(&eid);
        let old_name = eid.short_name().to_string();
        self.rename_dialog = Some(RenameDialog {
            new_name: old_name.clone(),
            old_name,
            file,
        });
    }

    fn delete_selected(&mut self) {
        let target_id = match &self.selected_target {
            SelectedTarget::Node { element_id, .. } | SelectedTarget::Element(element_id) => {
                Some(element_id.clone())
            }
            _ => None,
        };
        let Some(eid) = target_id else {
            return;
        };
        let file = self.file_for_element(&eid);
        let kind_hint = if self.graph.packages.contains_key(&eid) {
            Some("package".to_string())
        } else if self.graph.views.contains_key(&eid) {
            Some("view".to_string())
        } else {
            None
        };
        self.execute_command(SyssurCommand::DeleteElement {
            file,
            kind_hint,
            name: eid.short_name().to_string(),
        });
        self.selected_target = SelectedTarget::None;
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings.settings_open {
            return;
        }
        let locale = self.settings.locale;
        let native_ppp = ctx.native_pixels_per_point().unwrap_or(1.0);
        let current_ppp = ctx.pixels_per_point();
        let mut open = self.settings.settings_open;

        egui::Window::new(tr(locale, TextKey::Settings))
            .open(&mut open)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(tr(locale, TextKey::DisplayAndHiDpi)).strong());
                ui.label(format!(
                    "{}: {:.2}x (effective: {:.2}x)",
                    tr(locale, TextKey::DetectedDpi),
                    native_ppp,
                    current_ppp
                ));
                ui.checkbox(&mut self.settings.auto_hidpi, tr(locale, TextKey::AutoHiDpi));
                ui.add(
                    egui::Slider::new(&mut self.settings.ui_scale, 0.8..=2.5)
                        .text(tr(locale, TextKey::UiScale))
                        .step_by(0.05),
                );
                ui.add(
                    egui::Slider::new(&mut self.settings.base_font_size, 11.0..=24.0)
                        .text(tr(locale, TextKey::BaseFontSize))
                        .step_by(0.5),
                );

                ui.separator();
                ui.label(RichText::new(tr(locale, TextKey::Language)).strong());
                egui::ComboBox::from_id_salt("locale_selector")
                    .selected_text(self.settings.locale.display_name())
                    .show_ui(ui, |ui| {
                        for loc in Locale::ALL {
                            ui.selectable_value(&mut self.settings.locale, loc, loc.display_name());
                        }
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(tr(locale, TextKey::ResetDefaults)).clicked() {
                        let keep_open = self.settings.settings_open;
                        let keep_locale = self.settings.locale;
                        self.settings = AppSettings::default();
                        self.settings.settings_open = keep_open;
                        self.settings.locale = keep_locale;
                    }
                    if ui.button(tr(locale, TextKey::Close)).clicked() {
                        self.settings.settings_open = false;
                    }
                });
            });

        self.settings.settings_open &= open;
    }

    fn show_rename_window(&mut self, ctx: &egui::Context) {
        let Some(mut dlg) = self.rename_dialog.take() else {
            return;
        };
        let locale = self.settings.locale;
        let mut keep = true;
        let mut confirmed = false;

        egui::Window::new(tr(locale, TextKey::RenameTitle))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("{} ->", dlg.old_name));
                let resp = ui.text_edit_singleline(&mut dlg.new_name);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }
                ui.horizontal(|ui| {
                    if ui.button(tr(locale, TextKey::Apply)).clicked() {
                        confirmed = true;
                    }
                    if ui.button(tr(locale, TextKey::Cancel)).clicked() {
                        keep = false;
                    }
                });
            });

        if confirmed {
            let trimmed = dlg.new_name.trim().to_string();
            if !trimmed.is_empty() && trimmed != dlg.old_name {
                self.execute_command(SyssurCommand::RenameElement {
                    file: dlg.file,
                    old_name: dlg.old_name,
                    new_name: trimmed,
                });
            }
        } else if keep {
            self.rename_dialog = Some(dlg);
        }
    }

    fn show_top_bar(&mut self, ui: &mut Ui) {
        let now = ui.input(|i| i.time);
        let locale = self.settings.locale;
        egui::Panel::top("syssur_top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new("SYSSUR")
                        .strong()
                        .color(Color32::from_rgb(110, 200, 245)),
                );
                ui.separator();

                ui.menu_button(
                    format!("{} ({})", tr(locale, TextKey::Project), self.project_name),
                    |ui| {
                        if ui.button(tr(locale, TextKey::RoverSystem)).clicked() {
                            self.load_preset(ProjectPreset::RoverSystem);
                            ui.close();
                        }
                        if ui.button(tr(locale, TextKey::DoorbellSystem)).clicked() {
                            self.load_preset(ProjectPreset::DoorbellSystem);
                            ui.close();
                        }
                        if ui.button(tr(locale, TextKey::NewBlankProject)).clicked() {
                            self.load_preset(ProjectPreset::BlankProject);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(tr(locale, TextKey::ImportSysml)).clicked() {
                            self.io_modal.kind = IoModalKind::Import;
                            self.io_modal.open = true;
                            ui.close();
                        }
                        if ui.button(tr(locale, TextKey::ExportSysml)).clicked() {
                            self.io_modal.kind = IoModalKind::Export;
                            self.io_modal.open = true;
                            ui.close();
                        }
                    },
                );

                ui.toggle_value(&mut self.explorer_open, tr(locale, TextKey::ToggleExplorer));

                ui.separator();
                if ui
                    .add_enabled(
                        self.history.can_undo(),
                        egui::Button::new(tr(locale, TextKey::Undo)),
                    )
                    .clicked()
                {
                    self.undo();
                }
                if ui
                    .add_enabled(
                        self.history.can_redo(),
                        egui::Button::new(tr(locale, TextKey::Redo)),
                    )
                    .clicked()
                {
                    self.redo();
                }

                ui.separator();
                let (dot_color, conn_label) = match self.conn_status {
                    ConnStatus::Online => {
                        (Color32::from_rgb(90, 215, 130), tr(locale, TextKey::Online))
                    }
                    ConnStatus::Connecting => (
                        Color32::from_rgb(240, 195, 85),
                        tr(locale, TextKey::Syncing),
                    ),
                    ConnStatus::Offline => (
                        Color32::from_rgb(155, 160, 170),
                        tr(locale, TextKey::Offline),
                    ),
                };
                ui.menu_button(
                    RichText::new(format!("● {} ({conn_label})", tr(locale, TextKey::SyncAndGit)))
                        .color(dot_color),
                    |ui| {
                        ui.set_min_width(260.0);
                        ui.label("Server WebSocket URL:");
                        ui.add(
                            TextEdit::singleline(&mut self.ws_url)
                                .desired_width(250.0)
                                .hint_text("ws://127.0.0.1:8090/ws/projects/rover-system"),
                        );
                        ui.horizontal(|ui| {
                            if self.ws_client.is_none() {
                                if ui.button(tr(locale, TextKey::Connect)).clicked() {
                                    self.connect_ws(now);
                                }
                            } else if ui.button(tr(locale, TextKey::Disconnect)).clicked() {
                                self.disconnect_ws();
                            }
                            if ui
                                .add_enabled(
                                    self.conn_status == ConnStatus::Online,
                                    egui::Button::new(tr(locale, TextKey::CommitGitSnapshot)),
                                )
                                .clicked()
                            {
                                let summary = self
                                    .history
                                    .entries()
                                    .last()
                                    .map(|e| format!("Syssur: {}", e.summary))
                                    .unwrap_or_else(|| "Syssur model snapshot".to_string());
                                self.send_sync_msg(&SyncMessage::CommitSnapshot {
                                    message: summary,
                                });
                                self.status_line = "Requested Git snapshot commit...".to_string();
                                ui.close();
                            }
                        });
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(
                        &mut self.inspector_open,
                        tr(locale, TextKey::ToggleInspector),
                    );
                    if ui
                        .selectable_label(
                            self.settings.settings_open,
                            tr(locale, TextKey::Settings),
                        )
                        .clicked()
                    {
                        self.settings.settings_open = !self.settings.settings_open;
                    }
                });
            });
        });
    }

    fn show_bottom_bar(&mut self, ui: &mut Ui) {
        let locale = self.settings.locale;
        egui::Panel::bottom("syssur_bottom_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(tr(locale, TextKey::Cmd));
                let resp = ui.add(
                    TextEdit::singleline(&mut self.command_input)
                        .hint_text("part x : T | pkg P | state S | use case U | del X")
                        .desired_width(300.0),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let raw = self.command_input.trim().to_string();
                    self.run_quick_cli(&raw);
                    self.command_input.clear();
                }

                ui.separator();
                let error_count = self
                    .graph
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Error)
                    .count();
                let warning_count = self
                    .graph
                    .diagnostics
                    .iter()
                    .filter(|d| d.severity == DiagnosticSeverity::Warning)
                    .count();
                let diag_color = if error_count > 0 {
                    Color32::from_rgb(245, 115, 115)
                } else if warning_count > 0 {
                    Color32::from_rgb(240, 190, 85)
                } else {
                    Color32::from_rgb(100, 210, 145)
                };
                let diag_text = if error_count == 0 && warning_count == 0 {
                    format!("{}: 0", tr(locale, TextKey::Diagnostics))
                } else {
                    format!(
                        "{}: {}E / {}W",
                        tr(locale, TextKey::Diagnostics),
                        error_count,
                        warning_count
                    )
                };
                if ui
                    .add(egui::Button::new(RichText::new(diag_text).color(diag_color)).frame(false))
                    .clicked()
                {
                    self.inspector_open = true;
                    self.inspector_tab = InspectorTab::Code;
                }

                ui.separator();
                let latest = self
                    .history
                    .entries()
                    .last()
                    .map(|e| e.summary.as_str())
                    .unwrap_or("Ready");
                ui.label(
                    RichText::new(format!(
                        "{}: {latest} | {}",
                        tr(locale, TextKey::History),
                        self.status_line
                    ))
                    .weak(),
                );
            });
        });
    }

    fn run_quick_cli(&mut self, raw: &str) {
        let file = self.active_file.clone();
        let pkg = self.default_package_for_file(&file);
        if let Some(rest) = raw.strip_prefix("part ") {
            if let Some((name, ty)) = rest.split_once(':') {
                self.execute_command(SyssurCommand::CreatePartUsage {
                    file,
                    owner_def: self.active_root.short_name().to_string(),
                    name: name.trim().to_string(),
                    type_def: ty.trim().to_string(),
                });
            } else {
                self.execute_command(SyssurCommand::CreatePartUsage {
                    file,
                    owner_def: pkg,
                    name: rest.trim().to_string(),
                    type_def: String::new(),
                });
            }
        } else if let Some(name) = raw.strip_prefix("pkg ") {
            self.execute_command(SyssurCommand::CreatePackage {
                file,
                name: name.trim().to_string(),
            });
        } else if let Some(name) = raw.strip_prefix("view ") {
            self.execute_command(SyssurCommand::CreateView {
                file,
                package: pkg.clone(),
                name: name.trim().to_string(),
                view_kind: "GeneralView".to_string(),
                expose: vec![format!("{}::**", format_sysml_name(&pkg))],
            });
        } else if let Some(name) = raw.strip_prefix("action ") {
            self.execute_command(SyssurCommand::CreateAction {
                file,
                container: pkg,
                name: name.trim().to_string(),
                is_def: false,
            });
        } else if let Some(name) = raw.strip_prefix("state ") {
            self.execute_command(SyssurCommand::CreateSysmlElement {
                file,
                container: pkg,
                keyword: "state".to_string(),
                name: name.trim().to_string(),
                is_def: false,
                specializes: None,
                type_ref: None,
                doc: None,
            });
        } else if let Some(name) = raw.strip_prefix("req ") {
            self.execute_command(SyssurCommand::CreateRequirement {
                file,
                package: pkg,
                short_id: None,
                name: name.trim().to_string(),
                doc: String::new(),
                is_usage: true,
            });
        } else if let Some(name) = raw.strip_prefix("del ") {
            self.execute_command(SyssurCommand::DeleteElement {
                file,
                kind_hint: None,
                name: name.trim().to_string(),
            });
        }
    }

    fn show_left_explorer(&mut self, ui: &mut Ui) {
        if !self.explorer_open {
            return;
        }
        let locale = self.settings.locale;
        egui::Panel::left("syssur_model_explorer")
            .resizable(true)
            .default_size(255.0)
            .size_range(180.0..=440.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr(locale, TextKey::TabExplorer)).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .small_button("+")
                            .on_hover_text(tr(locale, TextKey::NewPackage))
                            .clicked()
                        {
                            let idx = self.graph.packages.len() + 1;
                            self.execute_command(SyssurCommand::CreatePackage {
                                file: self.active_file.clone(),
                                name: format!("Package{idx}"),
                            });
                        }
                    });
                });
                ui.separator();

                let mut pending_cmd: Option<SyssurCommand> = None;
                let mut pending_catalog: Option<(&'static SysmlCatalogItem, Option<String>, String)> =
                    None;
                let mut open_view_id: Option<ElementId> = None;
                let mut open_root_id: Option<(ElementId, ViewKind)> = None;
                let mut select_elem: Option<ElementId> = None;
                let mut rename_req: Option<(ElementId, String)> = None;

                ScrollArea::vertical().show(ui, |ui| {
                    let mut root_packages: Vec<_> = self
                        .graph
                        .packages
                        .values()
                        .filter(|p| {
                            !self.graph.packages.keys().any(|other| {
                                other != &p.id
                                    && p.id
                                        .as_str()
                                        .starts_with(&format!("{}::", other.as_str()))
                            })
                        })
                        .cloned()
                        .collect();
                    root_packages.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.id.cmp(&b.id)));
                    for pkg in root_packages {
                        render_explorer_package(
                            ui,
                            &self.graph,
                            &pkg,
                            &self.selected_target,
                            &mut self.explorer_search,
                            &mut select_elem,
                            &mut open_view_id,
                            &mut open_root_id,
                            &mut rename_req,
                            &mut pending_catalog,
                            &mut pending_cmd,
                            locale,
                        );
                    }

                    ui.separator();
                    ui.collapsing("Contextual Instance Tree", |ui| {
                        if let Some(tree) =
                            InstanceNode::expand_from_graph(&self.graph, &self.active_root, 4)
                        {
                            let mut sel_path = match &self.selected_target {
                                SelectedTarget::Node { path, .. } => Some(path.clone()),
                                _ => None,
                            };
                            render_instance_tree_ui(ui, &tree, &mut sel_path);
                            if let Some(p) = sel_path {
                                if let Some(leaf) = p.leaf().cloned() {
                                    self.selected_target = SelectedTarget::Node {
                                        path: p,
                                        element_id: leaf,
                                    };
                                }
                            }
                        }
                    });
                });

                if let Some(elem) = select_elem {
                    self.selected_target = SelectedTarget::Element(elem);
                }
                if let Some(vid) = open_view_id {
                    self.open_or_focus_view(&vid);
                }
                if let Some((rid, rkind)) = open_root_id {
                    self.open_or_focus_root(&rid, rkind);
                }
                if let Some((eid, file)) = rename_req {
                    let old_name = eid.short_name().to_string();
                    self.rename_dialog = Some(RenameDialog {
                        new_name: old_name.clone(),
                        old_name,
                        file,
                    });
                }
                if let Some((item, cname, file)) = pending_catalog {
                    self.place_catalog_item(item, cname, file);
                }
                if let Some(cmd) = pending_cmd {
                    self.execute_command(cmd);
                }
            });
    }

    fn show_right_inspector(&mut self, ui: &mut Ui) {
        if !self.inspector_open {
            return;
        }
        let locale = self.settings.locale;
        egui::Panel::right("syssur_right_inspector")
            .resizable(true)
            .default_size(340.0)
            .size_range(240.0..=560.0)
            .show(ui, |ui| {
                let active_layout = self
                    .active_view_tab
                    .and_then(|id| self.view_tabs.get(&id))
                    .map(|t| &t.layout);
                if let Some(act) = show_inspector_panel(
                    ui,
                    &mut self.inspector_tab,
                    &self.selected_target,
                    &mut self.inspector_draft,
                    &self.graph,
                    active_layout,
                    &self.collab,
                    &self.active_file,
                    locale,
                ) {
                    match act {
                        InspectorAction::ApplyCommand(cmd) => self.execute_command(cmd),
                        InspectorAction::MovePort {
                            node_key,
                            port_name,
                            placement,
                        } => {
                            if let Some(tab) = self
                                .active_view_tab
                                .and_then(|tid| self.view_tabs.get_mut(&tid))
                            {
                                if let Some(pos) = placement {
                                    tab.port_overrides.insert((node_key, port_name), pos);
                                } else {
                                    tab.port_overrides.remove(&(node_key, port_name));
                                }
                                tab.recompute_layout_only();
                            }
                        }
                        InspectorAction::EditFileText { file, text } => {
                            self.apply_and_broadcast_text(&file, &text);
                        }
                        InspectorAction::FormatFile { file } => {
                            if let Some(doc) = self.collab.files.get(&file) {
                                let formatted = SysmlEngine::format_source(&doc.text());
                                self.apply_and_broadcast_text(&file, &formatted);
                                self.status_line = format!("Formatted {file}");
                            }
                        }
                        InspectorAction::SelectFile(file) => {
                            self.active_file = file;
                        }
                        InspectorAction::JumpToCode(file) => {
                            if self.collab.files.contains_key(&file) {
                                self.active_file = file;
                                self.inspector_tab = InspectorTab::Code;
                            }
                        }
                    }
                }
            });
    }
}

impl eframe::App for SyssurApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_display_settings(&ctx);
        self.handle_global_shortcuts(&ctx);

        let now = ui.input(|i| i.time);
        self.poll_ws(now);
        if self.conn_status != ConnStatus::Offline || self.auto_reconnect {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Order: Top & Bottom bars -> Left & Right fixed-px sidebars -> Central scaling DockArea
        self.show_top_bar(ui);
        self.show_bottom_bar(ui);
        self.show_left_explorer(ui);
        self.show_right_inspector(ui);

        self.show_settings_window(&ctx);
        self.show_rename_window(&ctx);
        let texts = self.collab.snapshot_texts();
        if let Some(io_act) = show_io_modal(
            &ctx,
            &mut self.io_modal,
            &self.active_file,
            &texts,
            self.settings.locale,
        ) {
            match io_act {
                IoAction::ImportFiles {
                    files,
                    replace_workspace,
                } => {
                    let count = files.len();
                    self.open_imported_files(files, replace_workspace);
                    self.status_line = format!("Imported {count} .sysml file(s)");
                }
                IoAction::Status(msg) => {
                    self.status_line = msg;
                }
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let mut dock = std::mem::replace(&mut self.center_dock, DockState::new(vec![]));
            let mut viewer = CenterTabViewer { app: self };
            DockArea::new(&mut dock)
                .style(Style::from_egui(ui.style().as_ref()))
                .show_inside(ui, &mut viewer);
            self.center_dock = dock;
        });
    }
}

struct CenterTabViewer<'a> {
    app: &'a mut SyssurApp,
}

impl TabViewer for CenterTabViewer<'_> {
    type Tab = CenterTab;

    fn id(&mut self, tab: &mut Self::Tab) -> Id {
        Id::unique(tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> WidgetText {
        let locale = self.app.settings.locale;
        match tab {
            CenterTab::View(id) => self
                .app
                .view_tabs
                .get(id)
                .map(|t| t.title.clone())
                .unwrap_or_else(|| tr(locale, TextKey::TabDiagram).to_string())
                .into(),
            CenterTab::Traceability => tr(locale, TextKey::TabTraceability).into(),
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        let locale = self.app.settings.locale;
        match *tab {
            CenterTab::View(tab_id) => {
                self.app.active_view_tab = Some(tab_id);
                let mut relayout_needed = false;
                let mut auto_layout_clicked = false;
                let mut fit_needed = false;

                if let Some(vtab) = self.app.view_tabs.get_mut(&tab_id) {
                    ui.horizontal(|ui| {
                        let scope_lbl = match &vtab.scope {
                            ViewScope::SysmlView(id) => format!("view {}", id.short_name()),
                            ViewScope::AdHocRoot(id) => format!("root {}", id.short_name()),
                        };
                        ui.label(RichText::new(scope_lbl).strong());
                        ui.separator();

                        for (kind, key) in [
                            (ViewKind::General, TextKey::General),
                            (ViewKind::Interconnection, TextKey::Interconnection),
                            (ViewKind::ActionFlow, TextKey::ActionFlow),
                            (ViewKind::StateTransition, TextKey::StateTransition),
                        ] {
                            if ui
                                .selectable_label(vtab.view_kind == kind, tr(locale, key))
                                .clicked()
                            {
                                vtab.view_kind = kind;
                                vtab.direction = default_direction_for_view(kind);
                                relayout_needed = true;
                                fit_needed = true;
                            }
                        }

                        ui.separator();
                        let dir_lbl = match vtab.direction {
                            LayoutDirection::TopToBottom => tr(locale, TextKey::OrientationTB),
                            LayoutDirection::LeftToRight => tr(locale, TextKey::OrientationLR),
                        };
                        if ui.button(dir_lbl).on_hover_text("Flip TB / LR").clicked() {
                            vtab.direction = match vtab.direction {
                                LayoutDirection::TopToBottom => LayoutDirection::LeftToRight,
                                LayoutDirection::LeftToRight => LayoutDirection::TopToBottom,
                            };
                            relayout_needed = true;
                            fit_needed = true;
                        }

                        if ui.button(tr(locale, TextKey::AutoLayout)).clicked() {
                            auto_layout_clicked = true;
                        }
                        if ui.button(tr(locale, TextKey::Fit)).clicked() {
                            vtab.needs_fit = true;
                        }

                        ui.toggle_value(&mut vtab.palette_open, tr(locale, TextKey::Palette));

                        if vtab.view_kind == ViewKind::Interconnection {
                            ui.separator();
                            ui.label(tr(locale, TextKey::Wire));
                            ui.selectable_value(
                                &mut self.app.wire_mode,
                                WireMode::Connect,
                                "connect",
                            );
                            ui.selectable_value(
                                &mut self.app.wire_mode,
                                WireMode::Interface,
                                "interface",
                            );
                        }
                    });
                    ui.separator();
                }

                if auto_layout_clicked {
                    if let Some(vtab) = self.app.view_tabs.get_mut(&tab_id) {
                        vtab.node_offsets.clear();
                        vtab.port_overrides.clear();
                        vtab.recompute_layout_only();
                        vtab.needs_fit = true;
                    }
                } else if relayout_needed {
                    self.app.reparse_and_relayout();
                }
                if fit_needed {
                    if let Some(vtab) = self.app.view_tabs.get_mut(&tab_id) {
                        vtab.needs_fit = true;
                    }
                }

                let active_file = self.app.active_file.clone();
                let package_names: Vec<String> = self
                    .app
                    .graph
                    .packages
                    .values()
                    .map(|p| p.name.clone())
                    .collect();
                let ctx = ui.ctx().clone();
                let maybe_action = if let Some(vtab) = self.app.view_tabs.get_mut(&tab_id) {
                    if vtab.needs_fit {
                        vtab.canvas.fit_to_bounds(&vtab.layout, ui.available_size());
                        vtab.needs_fit = false;
                    }
                    vtab.canvas.show(
                        ui,
                        &vtab.layout,
                        &vtab.node_meta,
                        &package_names,
                        vtab.view_kind,
                        vtab.direction,
                        vtab.palette_open,
                        &mut vtab.palette_search,
                        &mut vtab.active_tool,
                        &active_file,
                        locale,
                    )
                } else {
                    None
                };

                if let Some(act) = maybe_action {
                    self.app.handle_canvas_action(tab_id, act, &ctx);
                }
            }
            CenterTab::Traceability => {
                ScrollArea::vertical().show(ui, |ui| {
                    ui.label(RichText::new("Requirements, Satisfy & Verify Links, and Verdicts").strong());
                    let linked: BTreeSet<&str> = self
                        .app
                        .graph
                        .trace_links
                        .iter()
                        .filter(|l| {
                            l.kind == RelationshipKind::Satisfy
                                || l.kind == RelationshipKind::Verify
                        })
                        .map(|l| l.requirement.short_name())
                        .collect();

                    egui::Grid::new("trace_grid").striped(true).show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Requirement");
                        ui.strong("Link");
                        ui.strong("Linked Elements");
                        ui.strong("Constraint Verdict");
                        ui.end_row();

                        for req in self.app.graph.requirements.values() {
                            ui.label(req.short_id.as_deref().unwrap_or("-"));
                            if ui.selectable_label(false, &req.name).clicked() {
                                self.app.selected_target = SelectedTarget::Element(req.id.clone());
                            }
                            let is_linked = linked.contains(req.name.as_str());
                            ui.label(if is_linked { "Linked" } else { "Open" });
                            let targets: Vec<String> = self
                                .app
                                .graph
                                .trace_links
                                .iter()
                                .filter(|l| l.requirement.short_name() == req.name)
                                .map(|l| format!("{:?}: {}", l.kind, l.target))
                                .collect();
                            ui.label(if targets.is_empty() {
                                "-".to_string()
                            } else {
                                targets.join(", ")
                            });

                            let req_verdict = self.app.graph.verdicts.iter().find(|v| {
                                v.context.as_deref().is_some_and(|ctx| {
                                    let ctx_short = ctx.rsplit("::").next().unwrap_or(ctx);
                                    ctx_short.eq_ignore_ascii_case(&req.name)
                                })
                            });
                            match req_verdict.map(|v| &v.status) {
                                Some(VerificationStatus::Satisfied) => {
                                    ui.label(
                                        RichText::new("Satisfied")
                                            .color(Color32::from_rgb(90, 215, 130)),
                                    );
                                }
                                Some(VerificationStatus::Violated) => {
                                    ui.label(
                                        RichText::new("Violated")
                                            .color(Color32::from_rgb(245, 100, 100)),
                                    );
                                }
                                Some(VerificationStatus::Undecided(reason)) => {
                                    ui.label(
                                        RichText::new(format!("Undecided ({reason})"))
                                            .color(Color32::from_rgb(240, 190, 85)),
                                    );
                                }
                                None => {
                                    ui.label("-");
                                }
                            }
                            ui.end_row();
                        }
                    });

                    if !self.app.graph.verdicts.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("Evaluated Constraint Verdicts").strong());
                        egui::Grid::new("verdict_grid").striped(true).show(ui, |ui| {
                            ui.strong("Context");
                            ui.strong("File");
                            ui.strong("Verdict");
                            ui.end_row();

                            for v in &self.app.graph.verdicts {
                                ui.label(v.context.as_deref().unwrap_or(&v.name));
                                ui.label(&v.span.file);
                                match &v.status {
                                    VerificationStatus::Satisfied => {
                                        ui.label(
                                            RichText::new("Satisfied")
                                                .color(Color32::from_rgb(90, 215, 130)),
                                        );
                                    }
                                    VerificationStatus::Violated => {
                                        ui.label(
                                            RichText::new("Violated")
                                                .color(Color32::from_rgb(245, 100, 100)),
                                        );
                                    }
                                    VerificationStatus::Undecided(r) => {
                                        ui.label(
                                            RichText::new(format!("Undecided ({r})"))
                                                .color(Color32::from_rgb(240, 190, 85)),
                                        );
                                    }
                                }
                                ui.end_row();
                            }
                        });
                    }
                });
            }
        }
    }
}

fn is_direct_pkg_child(parent_id: &ElementId, child_id: &ElementId) -> bool {
    let prefix = format!("{}::", parent_id.as_str());
    child_id
        .as_str()
        .strip_prefix(&prefix)
        .is_some_and(|rest| !rest.contains("::"))
}

#[allow(clippy::too_many_arguments)]
fn render_explorer_package(
    ui: &mut Ui,
    graph: &DefinitionGraph,
    pkg: &PackageNode,
    selected_target: &SelectedTarget,
    explorer_search: &mut String,
    select_elem: &mut Option<ElementId>,
    open_view_id: &mut Option<ElementId>,
    open_root_id: &mut Option<(ElementId, ViewKind)>,
    rename_req: &mut Option<(ElementId, String)>,
    pending_catalog: &mut Option<(&'static SysmlCatalogItem, Option<String>, String)>,
    pending_cmd: &mut Option<SyssurCommand>,
    locale: Locale,
) {
    let header_resp = ui.collapsing(format!("pkg {}", pkg.name), |ui| {
        // Direct child sub-packages first
        let mut child_pkgs: Vec<_> = graph
            .packages
            .values()
            .filter(|p| is_direct_pkg_child(&pkg.id, &p.id))
            .cloned()
            .collect();
        child_pkgs.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.id.cmp(&b.id)));
        for child_pkg in child_pkgs {
            render_explorer_package(
                ui,
                graph,
                &child_pkg,
                selected_target,
                explorer_search,
                select_elem,
                open_view_id,
                open_root_id,
                rename_req,
                pending_catalog,
                pending_cmd,
                locale,
            );
        }

        // Views
        for v in graph
            .views
            .values()
            .filter(|v| is_direct_pkg_child(&pkg.id, &v.id))
        {
            let resp = ui.selectable_label(false, format!("view {}", v.name));
            if resp.clicked() {
                *open_view_id = Some(v.id.clone());
                *select_elem = Some(v.id.clone());
            }
            resp.context_menu(|ui| {
                if ui.button(tr(locale, TextKey::CtxRenameElement)).clicked() {
                    *rename_req = Some((v.id.clone(), v.span.file.clone()));
                    ui.close();
                }
                if ui
                    .button(
                        RichText::new(tr(locale, TextKey::DeleteElement))
                            .color(Color32::from_rgb(245, 115, 115)),
                    )
                    .clicked()
                {
                    *pending_cmd = Some(SyssurCommand::DeleteElement {
                        file: v.span.file.clone(),
                        kind_hint: Some(if v.is_usage { "view" } else { "view def" }.into()),
                        name: v.name.clone(),
                    });
                    ui.close();
                }
            });
        }

        // Part definitions
        for pd in graph
            .part_defs
            .values()
            .filter(|d| is_direct_pkg_child(&pkg.id, &d.id))
        {
            let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &pd.id);
            let resp = ui.dnd_drag_source(
                Id::unique(("exp_pd", pd.id.as_str())),
                pd.id.clone(),
                |ui| ui.selectable_label(selected, format!("part def {}", pd.name)),
            );
            if resp.inner.clicked() {
                *select_elem = Some(pd.id.clone());
                *open_root_id = Some((pd.id.clone(), ViewKind::General));
            }
            explorer_element_context_menu(
                &resp.inner,
                &pd.id,
                &pd.name,
                "part def",
                &pd.span.file,
                Some(ViewKind::General),
                pending_cmd,
                rename_req,
                open_root_id,
                locale,
            );
        }

        // Top-level part usages
        for pu in graph
            .top_level_parts
            .values()
            .filter(|p| is_direct_pkg_child(&pkg.id, &p.id))
        {
            render_explorer_part(
                ui,
                pu,
                true,
                selected_target,
                select_elem,
                rename_req,
                open_root_id,
                pending_cmd,
                locale,
            );
        }

        // Port definitions
        for pdef in graph
            .port_defs
            .values()
            .filter(|p| is_direct_pkg_child(&pkg.id, &p.id))
        {
            let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &pdef.id);
            let resp = ui.dnd_drag_source(
                Id::unique(("exp_pdef", pdef.id.as_str())),
                pdef.id.clone(),
                |ui| ui.selectable_label(selected, format!("port def {}", pdef.name)),
            );
            if resp.inner.clicked() {
                *select_elem = Some(pdef.id.clone());
            }
            explorer_element_context_menu(
                &resp.inner,
                &pdef.id,
                &pdef.name,
                "port def",
                &pdef.span.file,
                None,
                pending_cmd,
                rename_req,
                open_root_id,
                locale,
            );
        }

        // Actions
        for act in graph
            .actions
            .values()
            .filter(|a| is_direct_pkg_child(&pkg.id, &a.id))
        {
            render_explorer_action(
                ui,
                act,
                selected_target,
                select_elem,
                rename_req,
                open_root_id,
                pending_cmd,
                locale,
            );
        }

        // States
        for st in graph
            .states
            .values()
            .filter(|s| is_direct_pkg_child(&pkg.id, &s.id))
        {
            render_explorer_state(
                ui,
                st,
                selected_target,
                select_elem,
                rename_req,
                open_root_id,
                pending_cmd,
                locale,
            );
        }

        // Requirements
        for req in graph
            .requirements
            .values()
            .filter(|r| is_direct_pkg_child(&pkg.id, &r.id))
        {
            let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &req.id);
            let badge = req
                .short_id
                .as_deref()
                .map(|s| format!("[{s}] "))
                .unwrap_or_default();
            let resp = ui.dnd_drag_source(
                Id::unique(("exp_req", req.id.as_str())),
                req.id.clone(),
                |ui| ui.selectable_label(selected, format!("req {badge}{}", req.name)),
            );
            if resp.inner.clicked() {
                *select_elem = Some(req.id.clone());
            }
            explorer_element_context_menu(
                &resp.inner,
                &req.id,
                &req.name,
                if req.is_usage {
                    "requirement"
                } else {
                    "requirement def"
                },
                &req.span.file,
                None,
                pending_cmd,
                rename_req,
                open_root_id,
                locale,
            );
        }

        // Generic SysML v2 Clause 7 elements
        for elem in graph
            .generic_elements
            .values()
            .filter(|g| is_direct_pkg_child(&pkg.id, &g.id))
        {
            let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &elem.id);
            let tag = short_kind_tag(elem.kind, elem.is_def);
            let resp = ui.dnd_drag_source(
                Id::unique(("exp_gen", elem.id.as_str())),
                elem.id.clone(),
                |ui| ui.selectable_label(selected, format!("{tag} {}", elem.name)),
            );
            if resp.inner.clicked() {
                *select_elem = Some(elem.id.clone());
            }
            explorer_element_context_menu(
                &resp.inner,
                &elem.id,
                &elem.name,
                tag,
                &elem.span.file,
                None,
                pending_cmd,
                rename_req,
                open_root_id,
                locale,
            );
        }
    });

    if header_resp.header_response.clicked() {
        *select_elem = Some(pkg.id.clone());
    }
    header_resp.header_response.context_menu(|ui| {
        ui.set_min_width(240.0);
        if ui.button("Open Package Diagram").clicked() {
            *open_root_id = Some((pkg.id.clone(), ViewKind::General));
            ui.close();
        }
        ui.separator();
        ui.add(
            TextEdit::singleline(explorer_search).hint_text("Search SysML v2 to create..."),
        );
        let q = explorer_search.trim().to_string();
        ui.separator();

        if !q.is_empty() {
            if let Some(item) = show_create_submenus(ui, &q) {
                *pending_catalog = Some((item, Some(pkg.name.clone()), pkg.span.file.clone()));
                ui.close();
            }
        } else {
            ui.menu_button("Create in Package...", |ui| {
                if let Some(item) = show_create_submenus(ui, "") {
                    *pending_catalog = Some((item, Some(pkg.name.clone()), pkg.span.file.clone()));
                    ui.close();
                }
            });
            ui.menu_button("Transform Package...", |ui| {
                if ui.button("Toggle library package").clicked() {
                    *pending_cmd = Some(SyssurCommand::ToggleModifier {
                        file: pkg.span.file.clone(),
                        element_name: pkg.name.clone(),
                        modifier: "library".to_string(),
                    });
                    ui.close();
                }
            });
        }

        ui.separator();
        if ui.button(tr(locale, TextKey::CtxRenameElement)).clicked() {
            *rename_req = Some((pkg.id.clone(), pkg.span.file.clone()));
            ui.close();
        }
        if ui
            .button(
                RichText::new(tr(locale, TextKey::DeletePackage))
                    .color(Color32::from_rgb(245, 115, 115)),
            )
            .clicked()
        {
            *pending_cmd = Some(SyssurCommand::DeleteElement {
                file: pkg.span.file.clone(),
                kind_hint: Some("package".to_string()),
                name: pkg.name.clone(),
            });
            ui.close();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn explorer_element_context_menu(
    resp: &egui::Response,
    id: &ElementId,
    name: &str,
    kind_hint: &str,
    file: &str,
    inspect_kind: Option<ViewKind>,
    pending_cmd: &mut Option<SyssurCommand>,
    rename_req: &mut Option<(ElementId, String)>,
    open_root_id: &mut Option<(ElementId, ViewKind)>,
    locale: Locale,
) {
    resp.context_menu(|ui| {
        if let Some(vk) = inspect_kind {
            if ui.button(tr(locale, TextKey::CtxInspectAsRoot)).clicked() {
                *open_root_id = Some((id.clone(), vk));
                ui.close();
            }
        }
        ui.menu_button("Transform...", |ui| {
            if ui.button("Def <-> Usage").clicked() {
                *pending_cmd = Some(SyssurCommand::ToggleDefUsage {
                    file: file.to_string(),
                    element_name: name.to_string(),
                });
                ui.close();
            }
            for m in ["abstract", "variation", "variant", "individual", "ref"] {
                if ui.button(format!("Toggle {m}")).clicked() {
                    *pending_cmd = Some(SyssurCommand::ToggleModifier {
                        file: file.to_string(),
                        element_name: name.to_string(),
                        modifier: m.to_string(),
                    });
                    ui.close();
                }
            }
        });
        if ui.button(tr(locale, TextKey::CtxRenameElement)).clicked() {
            *rename_req = Some((id.clone(), file.to_string()));
            ui.close();
        }
        if ui
            .button(
                RichText::new(tr(locale, TextKey::DeleteElement))
                    .color(Color32::from_rgb(245, 115, 115)),
            )
            .clicked()
        {
            *pending_cmd = Some(SyssurCommand::DeleteElement {
                file: file.to_string(),
                kind_hint: Some(kind_hint.to_string()),
                name: name.to_string(),
            });
            ui.close();
        }
    });
}

fn short_kind_tag(kind: SysmlElementKind, is_def: bool) -> &'static str {
    match (kind, is_def) {
        (SysmlElementKind::Attribute, true) => "attr def",
        (SysmlElementKind::Attribute, false) => "attr",
        (SysmlElementKind::Enum, _) => "enum def",
        (SysmlElementKind::Item, true) => "item def",
        (SysmlElementKind::Item, false) => "item",
        (SysmlElementKind::Occurrence, true) => "occ def",
        (SysmlElementKind::Occurrence, false) => "occ",
        (SysmlElementKind::Interface, true) => "iface def",
        (SysmlElementKind::Interface, false) => "iface",
        (SysmlElementKind::Constraint, true) => "constr def",
        (SysmlElementKind::Constraint, false) => "constr",
        (SysmlElementKind::Concern, _) => "concern",
        (SysmlElementKind::Calc, true) => "calc def",
        (SysmlElementKind::Calc, false) => "calc",
        (SysmlElementKind::UseCase, true) => "use case def",
        (SysmlElementKind::UseCase, false) => "use case",
        (SysmlElementKind::AnalysisCase, _) => "analysis",
        (SysmlElementKind::VerificationCase, _) => "verif",
        (SysmlElementKind::Case, _) => "case",
        (SysmlElementKind::Viewpoint, _) => "viewpoint",
        (SysmlElementKind::Rendering, _) => "rendering",
        (SysmlElementKind::Metadata, _) => "metadata",
        (SysmlElementKind::Comment, _) => "comment",
        (SysmlElementKind::Alias, _) => "alias",
        _ => "elem",
    }
}

#[allow(clippy::too_many_arguments)]
fn render_explorer_part(
    ui: &mut Ui,
    part: &syssur_model::PartUsage,
    is_top_level: bool,
    selected_target: &SelectedTarget,
    select_elem: &mut Option<ElementId>,
    rename_req: &mut Option<(ElementId, String)>,
    open_root_id: &mut Option<(ElementId, ViewKind)>,
    pending_cmd: &mut Option<SyssurCommand>,
    locale: Locale,
) {
    let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &part.id);
    let label = match &part.type_def {
        Some(td) => format!("part {} : {}", part.name, td.short_name()),
        None => format!("part {}", part.name),
    };
    let inspect_kind = is_top_level.then_some(ViewKind::General);
    if part.nested_parts.is_empty() {
        let resp =
            ui.dnd_drag_source(Id::unique(("exp_part", part.id.as_str())), part.id.clone(), |ui| {
                ui.selectable_label(selected, label)
            });
        if resp.inner.clicked() {
            *select_elem = Some(part.id.clone());
            if is_top_level {
                *open_root_id = Some((part.id.clone(), ViewKind::General));
            }
        }
        explorer_element_context_menu(
            &resp.inner,
            &part.id,
            &part.name,
            "part",
            &part.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    } else {
        let resp = ui.collapsing(label, |ui| {
            for child in &part.nested_parts {
                render_explorer_part(
                    ui,
                    child,
                    false,
                    selected_target,
                    select_elem,
                    rename_req,
                    open_root_id,
                    pending_cmd,
                    locale,
                );
            }
        });
        if resp.header_response.clicked() {
            *select_elem = Some(part.id.clone());
            if is_top_level {
                *open_root_id = Some((part.id.clone(), ViewKind::General));
            }
        }
        explorer_element_context_menu(
            &resp.header_response,
            &part.id,
            &part.name,
            "part",
            &part.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_explorer_action(
    ui: &mut Ui,
    act: &ActionNode,
    selected_target: &SelectedTarget,
    select_elem: &mut Option<ElementId>,
    rename_req: &mut Option<(ElementId, String)>,
    open_root_id: &mut Option<(ElementId, ViewKind)>,
    pending_cmd: &mut Option<SyssurCommand>,
    locale: Locale,
) {
    let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &act.id);
    let prefix = match act.kind {
        ActionNodeKind::Action => "action",
        ActionNodeKind::Start => "start",
        ActionNodeKind::Done => "done",
        ActionNodeKind::Fork => "fork",
        ActionNodeKind::Join => "join",
        ActionNodeKind::Decide => "decide",
        ActionNodeKind::Merge => "merge",
    };
    let label = format!("{prefix} {}", act.name);
    let inspect_kind = (!act.children.is_empty()).then_some(ViewKind::ActionFlow);
    if act.children.is_empty() {
        let resp =
            ui.dnd_drag_source(Id::unique(("exp_act", act.id.as_str())), act.id.clone(), |ui| {
                ui.selectable_label(selected, label)
            });
        if resp.inner.clicked() {
            *select_elem = Some(act.id.clone());
        }
        explorer_element_context_menu(
            &resp.inner,
            &act.id,
            &act.name,
            prefix,
            &act.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    } else {
        let resp = ui.collapsing(label, |ui| {
            for child in &act.children {
                render_explorer_action(
                    ui,
                    child,
                    selected_target,
                    select_elem,
                    rename_req,
                    open_root_id,
                    pending_cmd,
                    locale,
                );
            }
        });
        if resp.header_response.clicked() {
            *select_elem = Some(act.id.clone());
            *open_root_id = Some((act.id.clone(), ViewKind::ActionFlow));
        }
        explorer_element_context_menu(
            &resp.header_response,
            &act.id,
            &act.name,
            prefix,
            &act.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_explorer_state(
    ui: &mut Ui,
    st: &StateNode,
    selected_target: &SelectedTarget,
    select_elem: &mut Option<ElementId>,
    rename_req: &mut Option<(ElementId, String)>,
    open_root_id: &mut Option<(ElementId, ViewKind)>,
    pending_cmd: &mut Option<SyssurCommand>,
    locale: Locale,
) {
    let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &st.id);
    let kw = if st.is_def { "state def" } else { "state" };
    let label = format!("{kw} {}", st.name);
    let inspect_kind = (!st.children.is_empty()).then_some(ViewKind::StateTransition);
    if st.children.is_empty() {
        let resp =
            ui.dnd_drag_source(Id::unique(("exp_st", st.id.as_str())), st.id.clone(), |ui| {
                ui.selectable_label(selected, label)
            });
        if resp.inner.clicked() {
            *select_elem = Some(st.id.clone());
        }
        explorer_element_context_menu(
            &resp.inner,
            &st.id,
            &st.name,
            kw,
            &st.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    } else {
        let resp = ui.collapsing(label, |ui| {
            for child in &st.children {
                render_explorer_state(
                    ui,
                    child,
                    selected_target,
                    select_elem,
                    rename_req,
                    open_root_id,
                    pending_cmd,
                    locale,
                );
            }
        });
        if resp.header_response.clicked() {
            *select_elem = Some(st.id.clone());
            *open_root_id = Some((st.id.clone(), ViewKind::StateTransition));
        }
        explorer_element_context_menu(
            &resp.header_response,
            &st.id,
            &st.name,
            kw,
            &st.span.file,
            inspect_kind,
            pending_cmd,
            rename_req,
            open_root_id,
            locale,
        );
    }
}

fn render_instance_tree_ui(
    ui: &mut Ui,
    node: &InstanceNode,
    selected: &mut Option<InstancePath>,
) {
    let badge = match &node.origin {
        FeatureOrigin::Inherited(_) => "^ ",
        FeatureOrigin::Subsetted(_) => ":> ",
        FeatureOrigin::Redefined(_) => ":>> ",
        FeatureOrigin::Owned => "",
    };
    let text = match &node.type_def {
        Some(td) => format!("{badge}{} : {}", node.label, td.short_name()),
        None => format!("{badge}{}", node.label),
    };

    if node.children.is_empty() {
        let is_sel = selected.as_ref() == Some(&node.path);
        if ui.selectable_label(is_sel, text).clicked() {
            *selected = Some(node.path.clone());
        }
    } else {
        ui.collapsing(text, |ui| {
            for child in &node.children {
                render_instance_tree_ui(ui, child, selected);
            }
        });
    }
}

fn build_layout_input_for_view(
    graph: &DefinitionGraph,
    scope: &ViewScope,
    view_kind: ViewKind,
    direction: LayoutDirection,
) -> (LayoutInput, BTreeMap<String, NodeMeta>) {
    let filter = match scope {
        ViewScope::SysmlView(vid) => graph.views.get(vid).map(|v| ViewFilter {
            expose_patterns: v.expose_patterns.clone(),
            include_relationships: v.include_relationships.clone(),
        }),
        ViewScope::AdHocRoot(_) => None,
    };

    let mut input = LayoutInput {
        direction,
        nodes: Vec::new(),
        edges: Vec::new(),
    };
    let mut meta = BTreeMap::new();

    match view_kind {
        ViewKind::ActionFlow => {
            build_action_flow_layout(graph, scope, filter.as_ref(), &mut input, &mut meta);
        }
        ViewKind::StateTransition => {
            build_state_transition_layout(graph, scope, filter.as_ref(), &mut input, &mut meta);
        }
        ViewKind::General | ViewKind::Interconnection => {
            build_structural_layout(
                graph,
                scope,
                view_kind,
                filter.as_ref(),
                &mut input,
                &mut meta,
            );
        }
    }

    (input, meta)
}

fn build_action_flow_layout(
    graph: &DefinitionGraph,
    scope: &ViewScope,
    filter: Option<&ViewFilter>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let mut roots: Vec<&ActionNode> = Vec::new();
    match scope {
        ViewScope::AdHocRoot(id) => {
            if let Some(a) = graph.resolve_action(id) {
                roots.push(a);
            }
        }
        ViewScope::SysmlView(_) => {
            for act in graph.actions.values() {
                if filter.is_none_or(|f| f.matches_element(&act.id)) {
                    roots.push(act);
                }
            }
        }
    }

    let mut path_by_name: BTreeMap<String, InstancePath> = BTreeMap::new();
    for root in roots {
        collect_action_nodes_recursive(root, None, &mut path_by_name, input, meta);
    }

    for (idx, succ) in graph.successions.iter().enumerate() {
        if let (Some(src), Some(dst)) = (
            path_by_name.get(&succ.source),
            path_by_name.get(&succ.target),
        ) {
            input.edges.push(LayoutEdgeSpec {
                id: format!("{}#{idx}", succ.id),
                source_node: src.clone(),
                source_port: None,
                target_node: dst.clone(),
                target_port: None,
                kind: RelationshipKind::Succession,
                label: None,
                guard: succ.guard.clone(),
                span: succ.span.clone(),
            });
        }
    }
}

fn collect_action_nodes_recursive(
    node: &ActionNode,
    parent: Option<&InstancePath>,
    path_by_name: &mut BTreeMap<String, InstancePath>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let path = match parent {
        Some(p) => p.push(node.id.clone()),
        None => InstancePath::root(node.id.clone()),
    };
    let (shape, width, height) = match node.kind {
        ActionNodeKind::Action => {
            let w = (node.name.len() as f32 * 8.2 + 44.0).clamp(150.0, 260.0);
            let h = if node.children.is_empty() { 48.0 } else { 90.0 };
            (NodeShape::ActionBox, w, h)
        }
        ActionNodeKind::Start => (NodeShape::StartCircle, 26.0, 26.0),
        ActionNodeKind::Done => (NodeShape::DoneBullseye, 28.0, 28.0),
        ActionNodeKind::Fork | ActionNodeKind::Join => (NodeShape::ForkJoinBar, 140.0, 14.0),
        ActionNodeKind::Decide => (NodeShape::DecideDiamond, 34.0, 34.0),
        ActionNodeKind::Merge => (NodeShape::MergeDiamond, 34.0, 34.0),
    };

    path_by_name.insert(node.name.clone(), path.clone());
    meta.insert(
        path.canonical_key(),
        NodeMeta {
            type_name: None,
            type_def: Some(node.id.clone()),
            source_file: node.span.file.clone(),
            origin: FeatureOrigin::Owned,
            attributes: vec![],
            short_id: None,
            doc: node.doc.clone(),
            stereotype: format!("«{:?}»", node.kind),
        },
    );
    input.nodes.push(LayoutNodeSpec {
        path: path.clone(),
        parent: parent.cloned(),
        label: node.name.clone(),
        shape,
        width,
        height,
        ports: vec![],
        span: node.span.clone(),
    });

    for child in &node.children {
        collect_action_nodes_recursive(child, Some(&path), path_by_name, input, meta);
    }
}

fn build_state_transition_layout(
    graph: &DefinitionGraph,
    scope: &ViewScope,
    filter: Option<&ViewFilter>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let mut roots: Vec<&StateNode> = Vec::new();
    match scope {
        ViewScope::AdHocRoot(id) => {
            if let Some(s) = graph.resolve_state(id) {
                roots.push(s);
            }
        }
        ViewScope::SysmlView(_) => {
            for st in graph.states.values() {
                if filter.is_none_or(|f| f.matches_element(&st.id)) {
                    roots.push(st);
                }
            }
        }
    }

    let mut path_by_name: BTreeMap<String, InstancePath> = BTreeMap::new();
    for root in roots {
        collect_state_nodes_recursive(root, None, &mut path_by_name, input, meta);
    }

    for (idx, tr) in graph.transitions.iter().enumerate() {
        if tr.source == "start" && !path_by_name.contains_key("start") {
            let p = InstancePath::root(tr.owner_scope.child("start"));
            path_by_name.insert("start".to_string(), p.clone());
            input.nodes.push(LayoutNodeSpec {
                path: p.clone(),
                parent: path_by_name.get(tr.owner_scope.short_name()).cloned(),
                label: "start".to_string(),
                shape: NodeShape::StartCircle,
                width: 26.0,
                height: 26.0,
                ports: vec![],
                span: tr.span.clone(),
            });
            meta.insert(
                p.canonical_key(),
                NodeMeta {
                    type_name: None,
                    type_def: None,
                    source_file: tr.span.file.clone(),
                    origin: FeatureOrigin::Owned,
                    attributes: vec![],
                    short_id: None,
                    doc: None,
                    stereotype: "«start»".to_string(),
                },
            );
        }

        if let (Some(src), Some(dst)) = (
            path_by_name.get(&tr.source),
            path_by_name.get(&tr.target),
        ) {
            let mut label_parts = Vec::new();
            if let Some(ref trig) = tr.trigger {
                label_parts.push(trig.clone());
            }
            if let Some(ref eff) = tr.effect {
                label_parts.push(format!("/ {eff}"));
            }
            input.edges.push(LayoutEdgeSpec {
                id: format!("{}#{idx}", tr.id),
                source_node: src.clone(),
                source_port: None,
                target_node: dst.clone(),
                target_port: None,
                kind: RelationshipKind::Transition,
                label: if label_parts.is_empty() {
                    None
                } else {
                    Some(label_parts.join(" "))
                },
                guard: tr.guard.clone(),
                span: tr.span.clone(),
            });
        }
    }
}

fn collect_state_nodes_recursive(
    node: &StateNode,
    parent: Option<&InstancePath>,
    path_by_name: &mut BTreeMap<String, InstancePath>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let path = match parent {
        Some(p) => p.push(node.id.clone()),
        None => InstancePath::root(node.id.clone()),
    };
    let w = (node.name.len() as f32 * 8.2 + 44.0).clamp(140.0, 250.0);
    let h = if node.children.is_empty() { 48.0 } else { 90.0 };
    path_by_name.insert(node.name.clone(), path.clone());
    meta.insert(
        path.canonical_key(),
        NodeMeta {
            type_name: None,
            type_def: Some(node.id.clone()),
            source_file: node.span.file.clone(),
            origin: FeatureOrigin::Owned,
            attributes: vec![],
            short_id: None,
            doc: node.doc.clone(),
            stereotype: "«state»".to_string(),
        },
    );
    input.nodes.push(LayoutNodeSpec {
        path: path.clone(),
        parent: parent.cloned(),
        label: node.name.clone(),
        shape: NodeShape::StateBox,
        width: w,
        height: h,
        ports: vec![],
        span: node.span.clone(),
    });
    for child in &node.children {
        collect_state_nodes_recursive(child, Some(&path), path_by_name, input, meta);
    }
}

#[allow(clippy::too_many_arguments)]
fn populate_filtered_structural_nodes(
    graph: &DefinitionGraph,
    view_kind: ViewKind,
    include_root_parent: bool,
    filter: &ViewFilter,
    max_part_defs: usize,
    max_extras: usize,
    expanded_keys: &mut BTreeSet<String>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let part_def_roots: Vec<ElementId> = graph
        .part_defs
        .keys()
        .filter(|id| filter.matches_element(id))
        .cloned()
        .collect();

    let before_count = input.nodes.len();
    for root in &part_def_roots {
        if let Some(tree) = InstanceNode::expand_from_graph(graph, root, 2) {
            if !tree.children.is_empty() || graph.part_defs.len() <= 6 {
                collect_layout_nodes_filtered(
                    graph,
                    &tree,
                    None,
                    include_root_parent,
                    Some(filter),
                    expanded_keys,
                    input,
                    meta,
                );
            }
        }
    }

    for root_id in graph.top_level_parts.keys() {
        if let Some(tree) = InstanceNode::expand_from_graph(graph, root_id, 3) {
            collect_layout_nodes_filtered(
                graph,
                &tree,
                None,
                include_root_parent,
                Some(filter),
                expanded_keys,
                input,
                meta,
            );
        }
    }

    if input.nodes.len() == before_count {
        for root in part_def_roots.iter().take(max_part_defs) {
            if let Some(tree) = InstanceNode::expand_from_graph(graph, root, 2) {
                collect_layout_nodes_filtered(
                    graph,
                    &tree,
                    None,
                    include_root_parent,
                    Some(filter),
                    expanded_keys,
                    input,
                    meta,
                );
            }
        }
    }

    if view_kind != ViewKind::General {
        return;
    }

    let mut extra_count = 0usize;
    for req in graph.requirements.values() {
        if extra_count >= max_extras {
            break;
        }
        if filter.matches_element(&req.id) {
            let path = InstancePath::root(req.id.clone());
            if expanded_keys.insert(path.canonical_key()) {
                extra_count += 1;
                input.nodes.push(LayoutNodeSpec {
                    path: path.clone(),
                    parent: None,
                    label: req.name.clone(),
                    shape: NodeShape::RequirementCard,
                    width: 220.0,
                    height: if req.doc.is_some() { 68.0 } else { 48.0 },
                    ports: vec![],
                    span: req.span.clone(),
                });
                meta.insert(
                    path.canonical_key(),
                    NodeMeta {
                        type_name: None,
                        type_def: Some(req.id.clone()),
                        source_file: req.span.file.clone(),
                        origin: FeatureOrigin::Owned,
                        attributes: vec![],
                        short_id: req.short_id.clone(),
                        doc: req.doc.clone(),
                        stereotype: "«requirement»".to_string(),
                    },
                );
            }
        }
    }

    for st in graph.states.values() {
        if extra_count >= max_extras {
            break;
        }
        if filter.matches_element(&st.id) {
            let path = InstancePath::root(st.id.clone());
            if expanded_keys.insert(path.canonical_key()) {
                extra_count += 1;
                input.nodes.push(LayoutNodeSpec {
                    path: path.clone(),
                    parent: None,
                    label: st.name.clone(),
                    shape: NodeShape::StateBox,
                    width: 170.0,
                    height: 50.0,
                    ports: vec![],
                    span: st.span.clone(),
                });
                meta.insert(
                    path.canonical_key(),
                    NodeMeta {
                        type_name: None,
                        type_def: Some(st.id.clone()),
                        source_file: st.span.file.clone(),
                        origin: FeatureOrigin::Owned,
                        attributes: vec![],
                        short_id: None,
                        doc: st.doc.clone(),
                        stereotype: "«state»".to_string(),
                    },
                );
            }
        }
    }

    for elem in graph.generic_elements.values() {
        if extra_count >= max_extras {
            break;
        }
        if !filter.matches_element(&elem.id) {
            continue;
        }
        let path = InstancePath::root(elem.id.clone());
        if !expanded_keys.insert(path.canonical_key()) {
            continue;
        }
        extra_count += 1;
        let (shape, stereo) = match elem.kind {
            SysmlElementKind::Constraint | SysmlElementKind::Concern => {
                (NodeShape::ConstraintCard, "«constraint»")
            }
            SysmlElementKind::UseCase
            | SysmlElementKind::AnalysisCase
            | SysmlElementKind::VerificationCase
            | SysmlElementKind::Case => (
                NodeShape::CaseOval,
                match elem.kind {
                    SysmlElementKind::UseCase => "«use case»",
                    SysmlElementKind::AnalysisCase => "«analysis»",
                    SysmlElementKind::VerificationCase => "«verification»",
                    _ => "«case»",
                },
            ),
            _ => (
                NodeShape::GenericElement,
                match elem.kind {
                    SysmlElementKind::Item => "«item»",
                    SysmlElementKind::Attribute => "«attribute»",
                    SysmlElementKind::Enum => "«enum»",
                    SysmlElementKind::Occurrence => "«occurrence»",
                    SysmlElementKind::Calc => "«calc»",
                    SysmlElementKind::Interface => "«interface»",
                    SysmlElementKind::Viewpoint => "«viewpoint»",
                    SysmlElementKind::Metadata => "«metadata»",
                    SysmlElementKind::Comment => "«comment»",
                    _ => "«element»",
                },
            ),
        };
        input.nodes.push(LayoutNodeSpec {
            path: path.clone(),
            parent: None,
            label: elem.name.clone(),
            shape,
            width: 185.0,
            height: if elem.doc.is_some() { 64.0 } else { 48.0 },
            ports: vec![],
            span: elem.span.clone(),
        });
        meta.insert(
            path.canonical_key(),
            NodeMeta {
                type_name: elem.type_ref.as_ref().map(|t| t.short_name().to_string()),
                type_def: Some(elem.id.clone()),
                source_file: elem.span.file.clone(),
                origin: FeatureOrigin::Owned,
                attributes: vec![],
                short_id: elem.short_id.clone(),
                doc: elem.doc.clone(),
                stereotype: stereo.to_string(),
            },
        );
    }
}

fn build_structural_layout(
    graph: &DefinitionGraph,
    scope: &ViewScope,
    view_kind: ViewKind,
    filter: Option<&ViewFilter>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let mut expanded_keys = BTreeSet::new();
    let include_root_parent = view_kind == ViewKind::Interconnection;

    match scope {
        ViewScope::SysmlView(_) => {
            let f = filter.cloned().unwrap_or_default();
            populate_filtered_structural_nodes(
                graph,
                view_kind,
                include_root_parent,
                &f,
                48,
                usize::MAX,
                &mut expanded_keys,
                input,
                meta,
            );
        }
        ViewScope::AdHocRoot(root_id) => {
            if let Some(tree) = InstanceNode::expand_from_graph(graph, root_id, 3) {
                collect_layout_nodes_filtered(
                    graph,
                    &tree,
                    None,
                    include_root_parent,
                    None,
                    &mut expanded_keys,
                    input,
                    meta,
                );
            } else if graph.packages.contains_key(root_id) {
                let pkg_filter = ViewFilter {
                    expose_patterns: vec![format!("{}::**", root_id.as_str())],
                    include_relationships: Vec::new(),
                };
                populate_filtered_structural_nodes(
                    graph,
                    view_kind,
                    include_root_parent,
                    &pkg_filter,
                    48,
                    48,
                    &mut expanded_keys,
                    input,
                    meta,
                );
            }
        }
    }

    let visible_by_short: BTreeMap<String, InstancePath> = input
        .nodes
        .iter()
        .map(|n| (n.label.clone(), n.path.clone()))
        .collect();

    for (idx, conn) in graph.connections.iter().enumerate() {
        if let Some(f) = filter {
            if !f.allows_relationship(conn.kind) {
                continue;
            }
        }
        let (src_part, src_port) = split_port_ref(&conn.source_path);
        let (dst_part, dst_port) = split_port_ref(&conn.target_path);
        let src_short = src_part.rsplit('.').next().unwrap_or(src_part);
        let dst_short = dst_part.rsplit('.').next().unwrap_or(dst_part);
        if let (Some(src_node), Some(dst_node)) = (
            visible_by_short.get(src_short),
            visible_by_short.get(dst_short),
        ) {
            input.edges.push(LayoutEdgeSpec {
                id: format!("{}#{idx}", conn.id),
                source_node: src_node.clone(),
                source_port: src_port.map(str::to_string),
                target_node: dst_node.clone(),
                target_port: dst_port.map(str::to_string),
                kind: conn.kind,
                label: conn.name.clone().or_else(|| match conn.kind {
                    RelationshipKind::Flow => Some("«flow»".into()),
                    RelationshipKind::Allocate => Some("«allocate»".into()),
                    RelationshipKind::Dependency => Some("«dependency»".into()),
                    RelationshipKind::Binding => Some("=".into()),
                    _ => None,
                }),
                guard: None,
                span: conn.span.clone(),
            });
        }
    }

    if view_kind == ViewKind::General {
        for (idx, link) in graph.trace_links.iter().enumerate() {
            if link.kind != RelationshipKind::Satisfy && link.kind != RelationshipKind::Verify {
                continue;
            }
            let target_leaf = link.target.rsplit('.').next().unwrap_or(&link.target);
            let req_short = link.requirement.short_name();
            if let (Some(part_node), Some(req_node)) = (
                visible_by_short.get(target_leaf),
                visible_by_short.get(req_short),
            ) {
                let lbl = if link.kind == RelationshipKind::Verify {
                    "«verify»"
                } else {
                    "«satisfy»"
                };
                input.edges.push(LayoutEdgeSpec {
                    id: format!("trace#{idx}"),
                    source_node: part_node.clone(),
                    source_port: None,
                    target_node: req_node.clone(),
                    target_port: None,
                    kind: link.kind,
                    label: Some(lbl.to_string()),
                    guard: None,
                    span: link.span.clone(),
                });
            }
        }

        for (idx, pd) in graph.part_defs.values().enumerate() {
            if let Some(sub_node) = visible_by_short.get(&pd.name) {
                for sup in &pd.specializes {
                    if let Some(sup_node) = visible_by_short.get(sup.short_name()) {
                        input.edges.push(LayoutEdgeSpec {
                            id: format!("spec#{idx}#{}", sup.short_name()),
                            source_node: sub_node.clone(),
                            source_port: None,
                            target_node: sup_node.clone(),
                            target_port: None,
                            kind: RelationshipKind::Specialization,
                            label: Some(":>".to_string()),
                            guard: None,
                            span: pd.span.clone(),
                        });
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_layout_nodes_filtered(
    graph: &DefinitionGraph,
    node: &InstanceNode,
    parent_path: Option<&InstancePath>,
    is_compound_root: bool,
    filter: Option<&ViewFilter>,
    seen_keys: &mut BTreeSet<String>,
    input: &mut LayoutInput,
    meta: &mut BTreeMap<String, NodeMeta>,
) {
    let self_matches = filter.is_none_or(|f| {
        node.path.leaf().is_some_and(|leaf| {
            let type_ok = node.type_def.as_ref().is_some_and(|td| f.matches_element(td));
            f.matches_element(leaf) || type_ok
        })
    });

    if self_matches {
        let key = node.path.canonical_key();
        if seen_keys.insert(key.clone()) {
            let attributes: Vec<String> = node
                .attributes
                .iter()
                .map(|a| a.display_label())
                .collect();

            let ports: Vec<LayoutPortSpec> = node
                .ports
                .iter()
                .map(|p| LayoutPortSpec {
                    name: p.name.clone(),
                    direction: p.direction,
                    is_conjugated: p.is_conjugated,
                    type_label: p.port_def.as_ref().map(|d| d.short_name().to_string()),
                    span: p.span.clone(),
                })
                .collect();

            let height =
                64.0 + (attributes.len().min(3) as f32) * 16.0 + (ports.len() as f32) * 8.0;
            input.nodes.push(LayoutNodeSpec {
                path: node.path.clone(),
                parent: if is_compound_root {
                    None
                } else {
                    parent_path.cloned()
                },
                label: node.label.clone(),
                shape: NodeShape::PartBlock,
                width: 190.0,
                height,
                ports,
                span: node.span.clone(),
            });

            let source_file = node
                .type_def
                .as_ref()
                .and_then(|td| graph.resolve_part_def(td))
                .map(|pd| pd.span.file.clone())
                .unwrap_or_else(|| node.span.file.clone());

            meta.insert(
                key,
                NodeMeta {
                    type_name: node.type_def.as_ref().map(|d| d.short_name().to_string()),
                    type_def: node.type_def.clone(),
                    source_file,
                    origin: node.origin.clone(),
                    attributes,
                    short_id: None,
                    doc: None,
                    stereotype: "«part»".to_string(),
                },
            );
        }
    }

    let next_parent = if self_matches {
        if is_compound_root || parent_path.is_none() {
            Some(&node.path)
        } else {
            parent_path
        }
    } else {
        parent_path
    };

    for child in &node.children {
        collect_layout_nodes_filtered(
            graph,
            child,
            next_parent,
            false,
            filter,
            seen_keys,
            input,
            meta,
        );
    }
}

fn split_port_ref(path: &str) -> (&str, Option<&str>) {
    match path.rsplit_once('.') {
        Some((part, port)) => (part, Some(port)),
        None => (path, None),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "Syssur — SysML v2 Systems Modeling",
        options,
        Box::new(|_cc| Ok(Box::new(SyssurApp::default()))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    let web_options = eframe::WebOptions::default();
    wasm_bindgen_futures::spawn_local(async {
        let document = web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id("syssur_canvas")
            .expect("missing syssur_canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("not a canvas");
        let _ = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|_cc| Ok(Box::new(SyssurApp::default()))),
            )
            .await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doorbell_and_rover_presets_build_non_empty_layouts() {
        let app_rover = SyssurApp::default();
        assert!(!app_rover.view_tabs.is_empty());
        assert!(app_rover
            .view_tabs
            .values()
            .any(|t| !t.layout.nodes.is_empty()));

        let mut app_doorbell = SyssurApp::default();
        app_doorbell.load_preset(ProjectPreset::DoorbellSystem);
        let arrival = app_doorbell
            .view_tabs
            .values()
            .find(|t| t.title == "Visitor Arrival")
            .expect("Visitor Arrival tab");
        assert_eq!(arrival.view_kind, ViewKind::ActionFlow);
        assert_eq!(arrival.direction, LayoutDirection::TopToBottom);
        assert!(!arrival.layout.nodes.is_empty());
        assert!(!arrival.layout.edges.is_empty());
    }

    #[test]
    fn opening_ptc_25_04_31_displays_non_empty_diagram() {
        let mut app = SyssurApp::default();
        app.open_imported_files(
            vec![(
                "ptc-25-04-31.sysml".into(),
                include_str!("../../../docs/ptc-25-04-31.sysml").into(),
            )],
            true,
        );
        let active_tid = app.active_view_tab.expect("active view tab after import");
        let active_tab = app.view_tabs.get(&active_tid).expect("active tab state");
        assert!(
            active_tab.layout.nodes.len() >= 5,
            "expected >= 5 diagram nodes in initial tab, got {}",
            active_tab.layout.nodes.len()
        );
        assert!(
            !active_tab.layout.edges.is_empty(),
            "expected non-empty diagram edges in initial tab"
        );

        let safety_vid = app
            .graph
            .views
            .keys()
            .find(|id| id.short_name() == "vehiclePartsTree_Safety")
            .cloned()
            .expect("vehiclePartsTree_Safety view");
        app.open_or_focus_view(&safety_vid);
        let safety_tid = app.active_view_tab.expect("safety view tab");
        let safety_tab = app.view_tabs.get(&safety_tid).expect("safety tab state");
        assert!(
            !safety_tab.layout.nodes.is_empty(),
            "expected vehiclePartsTree_Safety view to display non-empty diagram nodes"
        );
    }
}
