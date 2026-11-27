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
use catalog::{paint_kind_icon, show_create_submenus, CatalogCategory, CreateContext, SysmlCatalogItem};
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
    node_size_overrides: BTreeMap<String, (f32, f32)>,
    port_overrides: BTreeMap<(String, String), PortPlacement>,
    layout: LayoutOutput,
    node_meta: BTreeMap<String, NodeMeta>,
    canvas: DiagramCanvas,
    needs_fit: bool,
    startup_autolayout_pending: bool,
    startup_stable_frames: u8,
    last_fit_viewport: egui::Vec2,
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
            node_size_overrides: BTreeMap::new(),
            port_overrides: BTreeMap::new(),
            layout: LayoutOutput::default(),
            node_meta: BTreeMap::new(),
            canvas: DiagramCanvas::default(),
            needs_fit: true,
            startup_autolayout_pending: true,
            startup_stable_frames: 0,
            last_fit_viewport: egui::Vec2::ZERO,
        }
    }

    fn recompute_layout_only(&mut self) {
        if self.node_size_overrides.is_empty() {
            self.layout = compute_layout_with_port_overrides(
                &self.base_input,
                &self.node_offsets,
                &self.port_overrides,
            );
        } else {
            let mut input = self.base_input.clone();
            for node in &mut input.nodes {
                if let Some(&(w, h)) = self.node_size_overrides.get(&node.path.canonical_key()) {
                    node.width = w;
                    node.height = h;
                }
            }
            self.layout = compute_layout_with_port_overrides(
                &input,
                &self.node_offsets,
                &self.port_overrides,
            );
        }
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

    fn resolve_container_owner(&self, id: &ElementId) -> Option<(String, String)> {
        if let Some(pd) = self.graph.resolve_part_def(id) {
            return Some((pd.name.clone(), pd.span.file.clone()));
        }
        if let Some(pu) = self.graph.top_level_parts.get(id) {
            return Some((pu.name.clone(), pu.span.file.clone()));
        }
        if let Some(pkg) = self.graph.packages.get(id) {
            return Some((pkg.name.clone(), pkg.span.file.clone()));
        }
        None
    }

    fn connect_ports(
        &mut self,
        source: PortEndpoint,
        target: PortEndpoint,
        rel_keyword: Option<String>,
    ) {
        let common_parent = source
            .node_path
            .parent()
            .and_then(|p| {
                target
                    .node_path
                    .parent()
                    .filter(|tp| *tp == p)
                    .and_then(|_| p.leaf().cloned())
            });

        let fallback_file = source
            .node_path
            .leaf()
            .map(|id| self.file_for_element(id))
            .unwrap_or_else(|| self.active_file.clone());

        let (owner_def, file) = common_parent
            .as_ref()
            .and_then(|id| self.resolve_container_owner(id))
            .or_else(|| {
                (!self.graph.views.contains_key(&self.active_root))
                    .then(|| self.resolve_container_owner(&self.active_root))
                    .flatten()
            })
            .or_else(|| {
                source
                    .node_path
                    .0
                    .first()
                    .and_then(|id| id.as_str().split("::").next())
                    .and_then(|pkg| {
                        self.graph
                            .packages
                            .values()
                            .find(|p| p.name == pkg)
                            .map(|p| (p.name.clone(), p.span.file.clone()))
                    })
            })
            .unwrap_or_else(|| self.target_package_for_active_view(&fallback_file));

        let src_ref = format!("{}.{}", source.node_label, source.port_name);
        let dst_ref = format!("{}.{}", target.node_label, target.port_name);

        let first_iface_def = || {
            self.graph
                .generic_elements
                .values()
                .find(|g| g.kind == SysmlElementKind::Interface && g.is_def)
                .map(|g| g.name.clone())
        };

        let cmd = match rel_keyword.as_deref() {
            Some("connection") => SyssurCommand::CreateConnection {
                file,
                owner_def,
                source: src_ref,
                target: dst_ref,
            },
            Some("interface") => {
                let idx = self.history.entries().len() + 1;
                if let Some(iface_def) = first_iface_def() {
                    SyssurCommand::CreateInterface {
                        file,
                        owner_def,
                        name: format!("link{idx}"),
                        interface_def: iface_def,
                        source: src_ref,
                        target: dst_ref,
                    }
                } else {
                    SyssurCommand::CreateRelationship {
                        file,
                        owner_scope: owner_def,
                        kind: "interface".to_string(),
                        source: src_ref,
                        target: dst_ref,
                        label: Some(format!("link{idx}")),
                        guard: None,
                    }
                }
            }
            Some(kw) => SyssurCommand::CreateRelationship {
                file,
                owner_scope: owner_def,
                kind: kw.to_string(),
                source: src_ref,
                target: dst_ref,
                label: None,
                guard: None,
            },
            None => match self.wire_mode {
                WireMode::Connect => SyssurCommand::CreateConnection {
                    file,
                    owner_def,
                    source: src_ref,
                    target: dst_ref,
                },
                WireMode::Interface => {
                    let idx = self.history.entries().len() + 1;
                    if let Some(iface_def) = first_iface_def() {
                        SyssurCommand::CreateInterface {
                            file,
                            owner_def,
                            name: format!("link{idx}"),
                            interface_def: iface_def,
                            source: src_ref,
                            target: dst_ref,
                        }
                    } else {
                        SyssurCommand::CreateRelationship {
                            file,
                            owner_scope: owner_def,
                            kind: "interface".to_string(),
                            source: src_ref,
                            target: dst_ref,
                            label: Some(format!("link{idx}")),
                            guard: None,
                        }
                    }
                }
            },
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

    fn target_package_for_active_view(&self, fallback_file: &str) -> (String, String) {
        if let Some(tab) = self.active_view_tab.and_then(|tid| self.view_tabs.get(&tid)) {
            match &tab.scope {
                ViewScope::AdHocRoot(root_id) => {
                    if let Some(pkg) = self.graph.packages.get(root_id) {
                        return (pkg.name.clone(), pkg.span.file.clone());
                    }
                }
                ViewScope::SysmlView(vid) => {
                    if let Some(view) = self.graph.views.get(vid) {
                        for pat in &view.expose_patterns {
                            let seg = pat
                                .split("::")
                                .next()
                                .unwrap_or("")
                                .trim()
                                .trim_matches('\'');
                            if let Some(pkg) = self.graph.packages.values().find(|p| p.name == seg) {
                                return (pkg.name.clone(), pkg.span.file.clone());
                            }
                        }
                    }
                }
            }
        }
        (
            self.default_package_for_file(fallback_file),
            fallback_file.to_string(),
        )
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
        let (pkg, file) = match container_name.clone() {
            Some(cn) => (cn, file),
            None => self.target_package_for_active_view(&file),
        };

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
                    file: file.clone(),
                    package: pkg,
                    name: name.clone(),
                    specializes: None,
                });
                let new_id = self
                    .graph
                    .part_defs
                    .values()
                    .find(|pd| pd.name == name && pd.span.file == file)
                    .or_else(|| self.graph.part_defs.values().find(|pd| pd.name == name))
                    .map(|pd| pd.id.clone());
                if let Some(new_id) = new_id {
                    self.selected_target = SelectedTarget::Element(new_id.clone());
                    let visible_in_active = self
                        .active_view_tab
                        .and_then(|tid| self.view_tabs.get(&tid))
                        .is_some_and(|tab| {
                            tab.layout
                                .nodes
                                .values()
                                .any(|n| n.label == name || n.path.leaf() == Some(&new_id))
                        });
                    if !visible_in_active {
                        self.open_or_focus_root(&new_id, ViewKind::General);
                    }
                }
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
            CanvasAction::ConnectPorts {
                source,
                target,
                rel_keyword,
            } => {
                self.connect_ports(source, target, rel_keyword);
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
                world_pos,
            } => {
                let before_keys: BTreeSet<String> = self
                    .view_tabs
                    .get(&tab_id)
                    .map(|t| t.layout.nodes.keys().cloned().collect())
                    .unwrap_or_default();
                let cname = container.and_then(|p| p.leaf().map(|l| l.short_name().to_string()));
                self.place_catalog_item(item, cname, file);
                if let Some(drop_world) = world_pos {
                    if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                        let new_nodes: Vec<(String, InstancePath, syssur_layout::Rect2)> = tab
                            .layout
                            .nodes
                            .iter()
                            .filter(|(k, _)| !before_keys.contains(*k))
                            .map(|(k, n)| (k.clone(), n.path.clone(), n.rect))
                            .collect();
                        if let Some((_, first_path, _)) = new_nodes.first().cloned() {
                            for (key, _, rect) in new_nodes {
                                let dx = drop_world.x - (rect.x + rect.w * 0.5);
                                let dy = drop_world.y - (rect.y + rect.h * 0.5);
                                let entry = tab
                                    .node_offsets
                                    .entry(key)
                                    .or_insert(Point2 { x: 0.0, y: 0.0 });
                                entry.x += dx;
                                entry.y += dy;
                            }
                            tab.canvas.selected = Some(first_path);
                            tab.startup_autolayout_pending = false;
                            tab.recompute_layout_only();
                        }
                    }
                }
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
                    tab.startup_autolayout_pending = false;
                    tab.recompute_layout_only();
                }
            }
            CanvasAction::MovePort {
                node_key,
                port_name,
                placement,
            } => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    tab.startup_autolayout_pending = false;
                    if let Some(pos) = placement {
                        tab.port_overrides.insert((node_key, port_name), pos);
                    } else {
                        tab.port_overrides.remove(&(node_key, port_name));
                    }
                    tab.recompute_layout_only();
                }
            }
            CanvasAction::ResizeNode {
                node_key,
                width,
                height,
                anchor_x,
                anchor_y,
                anchor_right,
                anchor_bottom,
            } => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    tab.startup_autolayout_pending = false;
                    tab.node_size_overrides
                        .insert(node_key.clone(), (width, height));
                    tab.recompute_layout_only();
                    if let Some(new_rect) = tab.layout.nodes.get(&node_key).map(|n| n.rect) {
                        let cur_ax = if anchor_right {
                            new_rect.x + new_rect.w
                        } else {
                            new_rect.x
                        };
                        let cur_ay = if anchor_bottom {
                            new_rect.y + new_rect.h
                        } else {
                            new_rect.y
                        };
                        let adj_x = anchor_x - cur_ax;
                        let adj_y = anchor_y - cur_ay;
                        if adj_x.abs() > 0.05 || adj_y.abs() > 0.05 {
                            let entry = tab
                                .node_offsets
                                .entry(node_key)
                                .or_insert(Point2 { x: 0.0, y: 0.0 });
                            entry.x += adj_x;
                            entry.y += adj_y;
                            tab.recompute_layout_only();
                        }
                    }
                }
            }
            CanvasAction::ResetNodeSize { node_key } => {
                if let Some(tab) = self.view_tabs.get_mut(&tab_id) {
                    tab.node_size_overrides.remove(&node_key);
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
                    self.inspector_tab = InspectorTab::Details;
                    self.selected_target = SelectedTarget::Node {
                        path,
                        element_id: leaf,
                    };
                }
            }
            CanvasAction::SelectPort {
                node_path,
                node_label,
                port_name,
            } => {
                self.inspector_tab = InspectorTab::Details;
                let node_key = node_path.canonical_key();
                let (port_type, direction, is_conjugated, side, file) = self
                    .view_tabs
                    .get(&tab_id)
                    .map(|tab| {
                        let p = tab
                            .layout
                            .nodes
                            .get(&node_key)
                            .and_then(|n| n.ports.iter().find(|p| p.name == port_name));
                        let f = tab
                            .node_meta
                            .get(&node_key)
                            .map(|m| m.source_file.clone())
                            .unwrap_or_else(|| self.active_file.clone());
                        (
                            p.and_then(|p| p.type_label.clone()),
                            p.map_or(syssur_model::PortDirection::InOut, |p| p.direction),
                            p.is_some_and(|p| p.is_conjugated),
                            p.map(|p| p.side),
                            f,
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            None,
                            syssur_model::PortDirection::InOut,
                            false,
                            None,
                            self.active_file.clone(),
                        )
                    });
                self.selected_target = SelectedTarget::Port {
                    node_key,
                    owner_label: node_label,
                    port_name,
                    port_type,
                    direction,
                    is_conjugated,
                    side,
                    file,
                };
            }
            CanvasAction::SelectEdge { index } => {
                if let Some(target) = self.resolve_edge_target(tab_id, index) {
                    self.inspector_tab = InspectorTab::Details;
                    self.selected_target = target;
                }
            }
            CanvasAction::DeleteEdge { index } => {
                if let Some(target) = self.resolve_edge_target(tab_id, index) {
                    self.delete_edge_target(&target);
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
                    tab.node_size_overrides.clear();
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
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
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
        if let SelectedTarget::Port {
            port_name, file, ..
        } = &self.selected_target
        {
            self.rename_dialog = Some(RenameDialog {
                old_name: port_name.clone(),
                new_name: port_name.clone(),
                file: file.clone(),
            });
            return;
        }
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

    fn resolve_edge_target(&self, tab_id: usize, index: usize) -> Option<SelectedTarget> {
        let tab = self.view_tabs.get(&tab_id)?;
        let edge = tab.layout.edges.get(index)?;
        if let Some(conn) = self
            .graph
            .connections
            .iter()
            .enumerate()
            .find(|(i, c)| format!("{}#{i}", c.id) == edge.id)
            .map(|(_, c)| c)
        {
            let (_, sp) = split_port_ref(&conn.source_path);
            let (_, tp) = split_port_ref(&conn.target_path);
            Some(SelectedTarget::Edge {
                name: conn.name.clone(),
                source: conn.source_path.clone(),
                target: conn.target_path.clone(),
                source_port: sp.map(str::to_string),
                target_port: tp.map(str::to_string),
                kind: conn.kind,
                type_name: conn
                    .interface_def
                    .as_ref()
                    .map(|d| d.short_name().to_string()),
                owner_scope: Some(conn.owner_scope.as_str().to_string()),
                trigger: None,
                guard: edge.guard.clone(),
                effect: None,
                span: (conn.span.start < conn.span.end).then(|| conn.span.clone()),
                file: conn.span.file.clone(),
            })
        } else if let Some(succ) = self
            .graph
            .successions
            .iter()
            .enumerate()
            .find(|(i, s)| format!("{}#{i}", s.id) == edge.id)
            .map(|(_, s)| s)
        {
            Some(SelectedTarget::Edge {
                name: None,
                source: succ.source.clone(),
                target: succ.target.clone(),
                source_port: None,
                target_port: None,
                kind: RelationshipKind::Succession,
                type_name: None,
                owner_scope: Some(succ.owner_scope.as_str().to_string()),
                trigger: None,
                guard: succ.guard.clone(),
                effect: None,
                span: (succ.span.start < succ.span.end).then(|| succ.span.clone()),
                file: succ.span.file.clone(),
            })
        } else if let Some(tr) = self
            .graph
            .transitions
            .iter()
            .enumerate()
            .find(|(i, t)| format!("{}#{i}", t.id) == edge.id)
            .map(|(_, t)| t)
        {
            Some(SelectedTarget::Edge {
                name: None,
                source: tr.source.clone(),
                target: tr.target.clone(),
                source_port: None,
                target_port: None,
                kind: RelationshipKind::Transition,
                type_name: None,
                owner_scope: Some(tr.owner_scope.as_str().to_string()),
                trigger: tr.trigger.clone(),
                guard: tr.guard.clone(),
                effect: tr.effect.clone(),
                span: (tr.span.start < tr.span.end).then(|| tr.span.clone()),
                file: tr.span.file.clone(),
            })
        } else if let Some(link) = self
            .graph
            .trace_links
            .iter()
            .enumerate()
            .find(|(i, _)| format!("trace#{i}") == edge.id)
            .map(|(_, l)| l)
        {
            Some(SelectedTarget::Edge {
                name: None,
                source: link.target.clone(),
                target: link.requirement.as_str().to_string(),
                source_port: None,
                target_port: None,
                kind: link.kind,
                type_name: None,
                owner_scope: None,
                trigger: None,
                guard: None,
                effect: None,
                span: (link.span.start < link.span.end).then(|| link.span.clone()),
                file: link.span.file.clone(),
            })
        } else if let Some(spec) = tab.base_input.edges.iter().find(|e| e.id == edge.id) {
            Some(SelectedTarget::Edge {
                name: spec.label.clone(),
                source: spec.source_node.to_string(),
                target: spec.target_node.to_string(),
                source_port: spec.source_port.clone(),
                target_port: spec.target_port.clone(),
                kind: spec.kind,
                type_name: None,
                owner_scope: None,
                trigger: None,
                guard: spec.guard.clone(),
                effect: None,
                span: (spec.span.start < spec.span.end).then(|| spec.span.clone()),
                file: spec.span.file.clone(),
            })
        } else {
            Some(SelectedTarget::Edge {
                name: edge.label.clone(),
                source: edge.id.clone(),
                target: edge.label.clone().unwrap_or_default(),
                source_port: None,
                target_port: None,
                kind: edge.kind,
                type_name: None,
                owner_scope: None,
                trigger: None,
                guard: edge.guard.clone(),
                effect: None,
                span: None,
                file: self.active_file.clone(),
            })
        }
    }

    fn clear_edge_selection(&mut self) {
        self.selected_target = SelectedTarget::None;
        for tab in self.view_tabs.values_mut() {
            tab.canvas.selected_edge = None;
        }
    }

    fn delete_edge_target(&mut self, target: &SelectedTarget) {
        if let SelectedTarget::Edge {
            kind,
            source,
            target: dst,
            span,
            file,
            ..
        } = target
        {
            if let Some(sp) = span {
                self.execute_command(SyssurCommand::DeleteSpan {
                    file: file.clone(),
                    start: sp.start,
                    end: sp.end,
                    label: format!("{kind:?} {source} -> {dst}"),
                });
                self.clear_edge_selection();
            } else {
                self.status_line = "Cannot delete inferred connector without source span".into();
            }
        }
    }

    fn delete_selected(&mut self) {
        if matches!(&self.selected_target, SelectedTarget::Edge { .. }) {
            let target = self.selected_target.clone();
            self.delete_edge_target(&target);
            return;
        }
        if let SelectedTarget::Port {
            port_name, file, ..
        } = &self.selected_target
        {
            self.execute_command(SyssurCommand::DeleteElement {
                file: file.clone(),
                kind_hint: Some("port".to_string()),
                name: port_name.clone(),
            });
            self.selected_target = SelectedTarget::None;
            return;
        }
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
                    self.inspector_tab = InspectorTab::Details;
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
                        InspectorAction::ApplyCommand(cmd) => {
                            let clears_edge = matches!(cmd, SyssurCommand::DeleteSpan { .. });
                            self.execute_command(cmd);
                            if clears_edge {
                                self.clear_edge_selection();
                            }
                        }
                        InspectorAction::ArmPortWire {
                            node_key,
                            port_name,
                        } => {
                            if let Some(tab) = self
                                .active_view_tab
                                .and_then(|tid| self.view_tabs.get_mut(&tid))
                            {
                                tab.canvas
                                    .arm_port_wire(&tab.layout, &node_key, &port_name);
                                self.status_line = format!("Click target port to wire from {port_name}");
                            }
                        }
                        InspectorAction::MovePort {
                            node_key,
                            port_name,
                            placement,
                        } => {
                            if let Some(tab) = self
                                .active_view_tab
                                .and_then(|tid| self.view_tabs.get_mut(&tid))
                            {
                                tab.startup_autolayout_pending = false;
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
                        vtab.node_size_overrides.clear();
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
                    let avail = ui.available_size();
                    if avail.x > 64.0 && avail.y > 64.0 {
                        if vtab.startup_autolayout_pending {
                            if (avail - vtab.last_fit_viewport).length() > 4.0 {
                                vtab.node_offsets.clear();
                                vtab.node_size_overrides.clear();
                                vtab.port_overrides.clear();
                                vtab.recompute_layout_only();
                                vtab.canvas.fit_to_bounds(&vtab.layout, avail);
                                vtab.last_fit_viewport = avail;
                                vtab.needs_fit = false;
                                vtab.startup_stable_frames = 0;
                            } else {
                                vtab.startup_stable_frames =
                                    vtab.startup_stable_frames.saturating_add(1);
                                if vtab.startup_stable_frames >= 3 {
                                    vtab.startup_autolayout_pending = false;
                                }
                            }
                        } else if vtab.needs_fit {
                            vtab.canvas.fit_to_bounds(&vtab.layout, avail);
                            vtab.last_fit_viewport = avail;
                            vtab.needs_fit = false;
                        }
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
    let pkg_selected = matches!(selected_target, SelectedTarget::Element(id) if id == &pkg.id);
    let header_resp = explorer_collapsing_with_icon(
        ui,
        ("exp_pkg", pkg.id.as_str()),
        true,
        pkg_selected,
        CatalogCategory::NamespacesAndAnnotations,
        "package",
        false,
        format!("pkg {}", pkg.name),
        |ui| {
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
            let selected = matches!(selected_target, SelectedTarget::Element(id) if id == &v.id);
            let resp = explorer_icon_row(
                ui,
                selected,
                CatalogCategory::ViewsAndMetadata,
                "view",
                !v.is_usage,
                format!("view {}", v.name),
                None,
            );
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
            let resp = explorer_icon_row(
                ui,
                selected,
                CatalogCategory::Structure,
                "part",
                true,
                format!("part def {}", pd.name),
                Some(pd.id.clone()),
            );
            if resp.clicked() {
                *select_elem = Some(pd.id.clone());
                *open_root_id = Some((pd.id.clone(), ViewKind::General));
            }
            explorer_element_context_menu(
                &resp,
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
            let resp = explorer_icon_row(
                ui,
                selected,
                CatalogCategory::PortsAndInterfaces,
                "port",
                true,
                format!("port def {}", pdef.name),
                Some(pdef.id.clone()),
            );
            if resp.clicked() {
                *select_elem = Some(pdef.id.clone());
            }
            explorer_element_context_menu(
                &resp,
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
            let resp = explorer_icon_row(
                ui,
                selected,
                CatalogCategory::CalcConstraintsRequirements,
                "requirement",
                !req.is_usage,
                format!("req {badge}{}", req.name),
                Some(req.id.clone()),
            );
            if resp.clicked() {
                *select_elem = Some(req.id.clone());
            }
            explorer_element_context_menu(
                &resp,
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
            let (cat, kw) = generic_element_icon_spec(elem.kind);
            let resp = explorer_icon_row(
                ui,
                selected,
                cat,
                kw,
                elem.is_def,
                format!("{tag} {}", elem.name),
                Some(elem.id.clone()),
            );
            if resp.clicked() {
                *select_elem = Some(elem.id.clone());
            }
            explorer_element_context_menu(
                &resp,
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

    if header_resp.clicked() {
        *select_elem = Some(pkg.id.clone());
    }
    egui::Popup::context_menu(&header_resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
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
            if let Some(item) = show_create_submenus(ui, &q, CreateContext::PackageExplorer) {
                *pending_catalog = Some((item, Some(pkg.name.clone()), pkg.span.file.clone()));
                ui.close();
            }
        } else {
            ui.menu_button("Create in Package...", |ui| {
                if let Some(item) = show_create_submenus(ui, "", CreateContext::PackageExplorer) {
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

fn generic_element_icon_spec(kind: SysmlElementKind) -> (CatalogCategory, &'static str) {
    match kind {
        SysmlElementKind::Attribute => (CatalogCategory::Structure, "attribute"),
        SysmlElementKind::Enum => (CatalogCategory::Structure, "enum"),
        SysmlElementKind::Item => (CatalogCategory::Structure, "item"),
        SysmlElementKind::Occurrence
        | SysmlElementKind::EventOccurrence
        | SysmlElementKind::Timeslice
        | SysmlElementKind::Snapshot
        | SysmlElementKind::RefUsage => (CatalogCategory::Structure, "occurrence"),
        SysmlElementKind::Interface
        | SysmlElementKind::Connection
        | SysmlElementKind::Allocation
        | SysmlElementKind::Flow => (CatalogCategory::PortsAndInterfaces, "interface"),
        SysmlElementKind::Constraint | SysmlElementKind::AssertConstraint => {
            (CatalogCategory::CalcConstraintsRequirements, "constraint")
        }
        SysmlElementKind::Concern => (CatalogCategory::CalcConstraintsRequirements, "concern"),
        SysmlElementKind::Calc => (CatalogCategory::CalcConstraintsRequirements, "calc"),
        SysmlElementKind::UseCase | SysmlElementKind::IncludeUseCase => {
            (CatalogCategory::CasesAndVerification, "use case")
        }
        SysmlElementKind::AnalysisCase => (CatalogCategory::CasesAndVerification, "analysis"),
        SysmlElementKind::VerificationCase => {
            (CatalogCategory::CasesAndVerification, "verification")
        }
        SysmlElementKind::Case
        | SysmlElementKind::Subject
        | SysmlElementKind::Actor
        | SysmlElementKind::Stakeholder => (CatalogCategory::CasesAndVerification, "case"),
        SysmlElementKind::TerminateAction => (CatalogCategory::ActionsAndControl, "terminate"),
        SysmlElementKind::PerformAction
        | SysmlElementKind::AcceptAction
        | SysmlElementKind::SendAction
        | SysmlElementKind::AssignAction
        | SysmlElementKind::IfAction
        | SysmlElementKind::LoopAction => (CatalogCategory::ActionsAndControl, "action"),
        SysmlElementKind::ExhibitState => (CatalogCategory::StatesAndTransitions, "state"),
        SysmlElementKind::Viewpoint => (CatalogCategory::ViewsAndMetadata, "viewpoint"),
        SysmlElementKind::Rendering => (CatalogCategory::ViewsAndMetadata, "rendering"),
        SysmlElementKind::Metadata | SysmlElementKind::TextualRep => {
            (CatalogCategory::ViewsAndMetadata, "metadata")
        }
        SysmlElementKind::Comment => (CatalogCategory::NamespacesAndAnnotations, "comment"),
        SysmlElementKind::Alias => (CatalogCategory::NamespacesAndAnnotations, "alias"),
        SysmlElementKind::Filter => (CatalogCategory::NamespacesAndAnnotations, "filter"),
    }
}

fn explorer_icon_row(
    ui: &mut Ui,
    selected: bool,
    category: CatalogCategory,
    keyword: &str,
    is_def: bool,
    label: impl Into<String>,
    payload: Option<ElementId>,
) -> egui::Response {
    let label = label.into();
    let sense = if payload.is_some() {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 19.0), sense);

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&resp, selected);
        if selected || resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 3.0, visuals.bg_fill);
        }
        let icon_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0, rect.center().y - 6.5),
            egui::vec2(13.0, 13.0),
        );
        paint_kind_icon(ui.painter(), icon_rect, category, keyword, is_def, false);
        ui.painter().text(
            egui::pos2(rect.left() + 20.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.5),
            visuals.text_color(),
        );
    }

    if let Some(p) = payload {
        resp.dnd_set_drag_payload(p);
    }
    resp
}

#[allow(clippy::too_many_arguments)]
fn explorer_collapsing_with_icon(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    default_open: bool,
    selected: bool,
    category: CatalogCategory,
    keyword: &str,
    is_def: bool,
    label: impl Into<String>,
    add_body: impl FnOnce(&mut Ui),
) -> egui::Response {
    let label = label.into();
    let id = ui.make_persistent_id(id_salt);
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let (_, header_inner, _) = state
        .show_header(ui, |ui| {
            explorer_icon_row(ui, selected, category, keyword, is_def, label, None)
        })
        .body(add_body);
    header_inner.inner
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
        let resp = explorer_icon_row(
            ui,
            selected,
            CatalogCategory::Structure,
            "part",
            false,
            label,
            Some(part.id.clone()),
        );
        if resp.clicked() {
            *select_elem = Some(part.id.clone());
            if is_top_level {
                *open_root_id = Some((part.id.clone(), ViewKind::General));
            }
        }
        explorer_element_context_menu(
            &resp,
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
        let header_resp = explorer_collapsing_with_icon(
            ui,
            ("exp_part", part.id.as_str()),
            false,
            selected,
            CatalogCategory::Structure,
            "part",
            false,
            label,
            |ui| {
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
            },
        );
        if header_resp.clicked() {
            *select_elem = Some(part.id.clone());
            if is_top_level {
                *open_root_id = Some((part.id.clone(), ViewKind::General));
            }
        }
        explorer_element_context_menu(
            &header_resp,
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
        let resp = explorer_icon_row(
            ui,
            selected,
            CatalogCategory::ActionsAndControl,
            prefix,
            false,
            label,
            Some(act.id.clone()),
        );
        if resp.clicked() {
            *select_elem = Some(act.id.clone());
        }
        explorer_element_context_menu(
            &resp,
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
        let header_resp = explorer_collapsing_with_icon(
            ui,
            ("exp_act", act.id.as_str()),
            false,
            selected,
            CatalogCategory::ActionsAndControl,
            prefix,
            false,
            label,
            |ui| {
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
            },
        );
        if header_resp.clicked() {
            *select_elem = Some(act.id.clone());
            *open_root_id = Some((act.id.clone(), ViewKind::ActionFlow));
        }
        explorer_element_context_menu(
            &header_resp,
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
        let resp = explorer_icon_row(
            ui,
            selected,
            CatalogCategory::StatesAndTransitions,
            "state",
            st.is_def,
            label,
            Some(st.id.clone()),
        );
        if resp.clicked() {
            *select_elem = Some(st.id.clone());
        }
        explorer_element_context_menu(
            &resp,
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
        let header_resp = explorer_collapsing_with_icon(
            ui,
            ("exp_st", st.id.as_str()),
            false,
            selected,
            CatalogCategory::StatesAndTransitions,
            "state",
            st.is_def,
            label,
            |ui| {
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
            },
        );
        if header_resp.clicked() {
            *select_elem = Some(st.id.clone());
            *open_root_id = Some((st.id.clone(), ViewKind::StateTransition));
        }
        explorer_element_context_menu(
            &header_resp,
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
            let (w, h) = fit_labeled_box_size("«action» ", &node.name, !node.children.is_empty(), 130.0);
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
            is_def: false,
            type_name: None,
            type_def: Some(node.id.clone()),
            specializes: None,
            multiplicity: None,
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
                    is_def: false,
                    type_name: None,
                    type_def: None,
                    specializes: None,
                    multiplicity: None,
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
    let (w, h) = fit_labeled_box_size("«state» ", &node.name, !node.children.is_empty(), 130.0);
    path_by_name.insert(node.name.clone(), path.clone());
    meta.insert(
        path.canonical_key(),
        NodeMeta {
            is_def: node.is_def,
            type_name: None,
            type_def: Some(node.id.clone()),
            specializes: None,
            multiplicity: None,
            source_file: node.span.file.clone(),
            origin: FeatureOrigin::Owned,
            attributes: vec![],
            short_id: None,
            doc: node.doc.clone(),
            stereotype: if node.is_def {
                "«state def»".to_string()
            } else {
                "«state»".to_string()
            },
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

fn collect_instantiated_part_defs(graph: &DefinitionGraph) -> BTreeSet<ElementId> {
    fn visit_usage(
        graph: &DefinitionGraph,
        u: &syssur_model::PartUsage,
        out: &mut BTreeSet<ElementId>,
    ) {
        if let Some(ref td) = u.type_def {
            out.insert(td.clone());
            if let Some(pd) = graph.resolve_part_def(td) {
                out.insert(pd.id.clone());
            }
        }
        for child in &u.nested_parts {
            visit_usage(graph, child, out);
        }
    }

    let mut out = BTreeSet::new();
    for pd in graph.part_defs.values() {
        for u in &pd.parts {
            visit_usage(graph, u, &mut out);
        }
    }
    for u in graph.top_level_parts.values() {
        visit_usage(graph, u, &mut out);
    }
    out
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
    let instantiated_types = collect_instantiated_part_defs(graph);

    let before_count = input.nodes.len();
    let mut shown_part_defs = 0usize;
    for root in &part_def_roots {
        if shown_part_defs >= max_part_defs {
            break;
        }
        if let Some(tree) = InstanceNode::expand_from_graph(graph, root, 2) {
            if !tree.children.is_empty()
                || !instantiated_types.contains(root)
                || graph.part_defs.len() <= 6
            {
                shown_part_defs += 1;
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
                let (width, height) = fit_requirement_card_size(
                    "«requirement»",
                    req.short_id.as_deref(),
                    &req.name,
                    req.doc.as_deref(),
                );
                input.nodes.push(LayoutNodeSpec {
                    path: path.clone(),
                    parent: None,
                    label: req.name.clone(),
                    shape: NodeShape::RequirementCard,
                    width,
                    height,
                    ports: vec![],
                    span: req.span.clone(),
                });
                meta.insert(
                    path.canonical_key(),
                    NodeMeta {
                        is_def: !req.is_usage,
                        type_name: None,
                        type_def: Some(req.id.clone()),
                        specializes: None,
                        multiplicity: None,
                        source_file: req.span.file.clone(),
                        origin: FeatureOrigin::Owned,
                        attributes: vec![],
                        short_id: req.short_id.clone(),
                        doc: req.doc.clone(),
                        stereotype: if req.is_usage {
                            "«requirement»".to_string()
                        } else {
                            "«requirement def»".to_string()
                        },
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
                let st_stereo = if st.is_def {
                    "«state def»".to_string()
                } else {
                    "«state»".to_string()
                };
                let (width, height) = fit_labeled_box_size(&st_stereo, &st.name, false, 130.0);
                input.nodes.push(LayoutNodeSpec {
                    path: path.clone(),
                    parent: None,
                    label: st.name.clone(),
                    shape: NodeShape::StateBox,
                    width,
                    height,
                    ports: vec![],
                    span: st.span.clone(),
                });
                meta.insert(
                    path.canonical_key(),
                    NodeMeta {
                        is_def: st.is_def,
                        type_name: None,
                        type_def: Some(st.id.clone()),
                        specializes: None,
                        multiplicity: None,
                        source_file: st.span.file.clone(),
                        origin: FeatureOrigin::Owned,
                        attributes: vec![],
                        short_id: None,
                        doc: st.doc.clone(),
                        stereotype: st_stereo,
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
        let type_name = elem.type_ref.as_ref().map(|t| t.short_name().to_string());
        let (width, height) = if shape == NodeShape::ConstraintCard {
            fit_requirement_card_size(
                stereo,
                elem.short_id.as_deref(),
                &elem.name,
                elem.doc.as_deref(),
            )
        } else {
            fit_generic_element_size(
                stereo,
                &elem.name,
                type_name.as_deref(),
                shape,
                elem.doc.is_some(),
            )
        };
        input.nodes.push(LayoutNodeSpec {
            path: path.clone(),
            parent: None,
            label: elem.name.clone(),
            shape,
            width,
            height,
            ports: vec![],
            span: elem.span.clone(),
        });
        meta.insert(
            path.canonical_key(),
            NodeMeta {
                is_def: elem.is_def,
                type_name,
                type_def: Some(elem.id.clone()),
                specializes: None,
                multiplicity: None,
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
                label: match (&conn.name, &conn.interface_def) {
                    (Some(n), Some(idef)) => Some(format!("{n} : {}", idef.short_name())),
                    (Some(n), None) => Some(n.clone()),
                    (None, Some(idef)) => Some(format!(": {}", idef.short_name())),
                    (None, None) => match conn.kind {
                        RelationshipKind::Flow => Some("«flow»".into()),
                        RelationshipKind::Allocate => Some("«allocate»".into()),
                        RelationshipKind::Dependency => Some("«dependency»".into()),
                        RelationshipKind::Binding => Some("=".into()),
                        _ => None,
                    },
                },
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
const MAX_VISIBLE_BLOCK_ATTRS: usize = 8;

fn find_part_multiplicity(graph: &DefinitionGraph, id: &ElementId) -> Option<u32> {
    let base = id.as_str().split('#').next().unwrap_or(id.as_str());
    let base_id = ElementId::new(base);
    fn search(u: &syssur_model::PartUsage, id: &ElementId) -> Option<u32> {
        if &u.id == id {
            return u.multiplicity;
        }
        u.nested_parts.iter().find_map(|c| search(c, id))
    }
    graph
        .top_level_parts
        .values()
        .find_map(|u| search(u, &base_id))
        .or_else(|| {
            graph
                .part_defs
                .values()
                .flat_map(|pd| pd.parts.iter())
                .find_map(|u| search(u, &base_id))
        })
}

fn format_part_stereotype(
    is_def: bool,
    mods: Option<&syssur_model::ElementModifiers>,
) -> String {
    let mut words = Vec::new();
    if let Some(m) = mods {
        if m.is_variation {
            words.push("variation");
        } else if m.is_variant {
            words.push("variant");
        }
        if m.is_abstract {
            words.push("abstract");
        }
        if m.is_individual {
            words.push("individual");
        }
        if m.is_ref {
            words.push("ref");
        }
    }
    words.push(if is_def { "part def" } else { "part" });
    format!("«{}»", words.join(" "))
}

fn fit_part_block_size(
    stereotype: &str,
    label: &str,
    type_name: Option<&str>,
    specializes: Option<&str>,
    multiplicity: Option<u32>,
    origin: &FeatureOrigin,
    attributes: &[String],
    port_count: usize,
) -> (f32, f32) {
    let stereo_w = stereotype.chars().count() as f32 * 6.8 + 26.0;
    let inh_len = usize::from(matches!(origin, FeatureOrigin::Inherited(_)));
    let suffix_len = specializes
        .map_or_else(
            || type_name.map_or(0, |tn| 3 + tn.chars().count()),
            |sp| 4 + sp.chars().count(),
        )
        + multiplicity.map_or(0, |m| 3 + m.to_string().len());
    let title_w = (inh_len + label.chars().count() + suffix_len) as f32 * 8.2 + 28.0;
    let origin_sub_len = match origin {
        FeatureOrigin::Subsetted(t) => 8 + t.short_name().chars().count(),
        FeatureOrigin::Redefined(t) => 10 + t.short_name().chars().count(),
        _ => 0,
    };
    let origin_w = if origin_sub_len > 0 {
        origin_sub_len as f32 * 6.8 + 26.0
    } else {
        0.0
    };
    let attr_w = attributes
        .iter()
        .take(MAX_VISIBLE_BLOCK_ATTRS)
        .map(|a| a.chars().count() as f32 * 7.4 + 26.0)
        .fold(0.0_f32, f32::max);
    let port_w = 120.0 + (port_count.saturating_sub(2) as f32) * 24.0;
    let width = stereo_w
        .max(title_w)
        .max(origin_w)
        .max(attr_w)
        .max(port_w)
        .clamp(124.0, 480.0);

    let name_comp_h = if origin_sub_len > 0 { 52.0 } else { 38.0 };
    let shown_attrs = attributes.len().min(MAX_VISIBLE_BLOCK_ATTRS);
    let attrs_comp_h = if shown_attrs > 0 {
        20.0 + shown_attrs as f32 * 15.0 + 8.0
    } else {
        4.0
    };
    let port_pad = (port_count.saturating_sub(2) as f32) * 8.0;
    let height = (name_comp_h + attrs_comp_h + port_pad).max(46.0);
    (width, height)
}

fn fit_requirement_card_size(
    stereo: &str,
    short_id: Option<&str>,
    label: &str,
    doc: Option<&str>,
) -> (f32, f32) {
    let stereo_w = stereo.chars().count() as f32 * 7.0 + 20.0;
    let title_len = short_id.map_or(0, |s| s.chars().count() + 3) + label.chars().count();
    let title_w = title_len as f32 * 8.2 + 22.0;
    let doc_w = doc.map_or(0.0, |d| {
        let len = d.chars().count();
        (len.min(46) + usize::from(len > 46) * 3) as f32 * 6.5 + 22.0
    });
    let width = stereo_w.max(title_w).max(doc_w).clamp(150.0, 440.0);
    let height = if doc.is_some() { 64.0 } else { 44.0 };
    (width, height)
}

fn fit_labeled_box_size(prefix: &str, label: &str, has_children: bool, min_w: f32) -> (f32, f32) {
    let text_len = prefix.chars().count() + label.chars().count();
    let width = (text_len as f32 * 8.0 + 28.0).clamp(min_w, 420.0);
    let height = if has_children { 86.0 } else { 44.0 };
    (width, height)
}

fn fit_generic_element_size(
    stereo: &str,
    label: &str,
    type_name: Option<&str>,
    shape: NodeShape,
    has_doc: bool,
) -> (f32, f32) {
    let stereo_w = stereo.chars().count() as f32 * 7.0 + 22.0;
    let title_len = label.chars().count() + type_name.map_or(0, |t| t.chars().count() + 3);
    let pad = if shape == NodeShape::CaseOval {
        36.0
    } else {
        22.0
    };
    let width = stereo_w.max(title_len as f32 * 8.0 + pad).clamp(130.0, 420.0);
    let height = if has_doc { 62.0 } else { 46.0 };
    (width, height)
}

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
                .map(|a| {
                    if matches!(a.origin, FeatureOrigin::Inherited(_)) {
                        format!("^{}", a.display_label())
                    } else {
                        a.display_label()
                    }
                })
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

            let leaf_id = node.path.leaf();
            let base_leaf = leaf_id.map(|leaf| {
                ElementId::new(leaf.as_str().split('#').next().unwrap_or(leaf.as_str()))
            });
            let is_def = node.path.0.len() == 1
                && leaf_id.is_some_and(|leaf| graph.part_defs.contains_key(leaf));
            let mods = base_leaf
                .as_ref()
                .and_then(|id| graph.element_modifiers.get(id));
            let stereotype = format_part_stereotype(is_def, mods);
            let specializes = if is_def {
                leaf_id
                    .and_then(|leaf| graph.part_defs.get(leaf))
                    .filter(|pd| !pd.specializes.is_empty())
                    .map(|pd| {
                        pd.specializes
                            .iter()
                            .map(|s| s.short_name())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
            } else {
                None
            };
            let multiplicity = if is_def {
                None
            } else {
                leaf_id.and_then(|leaf| find_part_multiplicity(graph, leaf))
            };
            let type_name = node.type_def.as_ref().map(|d| d.short_name().to_string());
            let (width, height) = fit_part_block_size(
                &stereotype,
                &node.label,
                type_name.as_deref(),
                specializes.as_deref(),
                multiplicity,
                &node.origin,
                &attributes,
                ports.len(),
            );
            input.nodes.push(LayoutNodeSpec {
                path: node.path.clone(),
                parent: if is_compound_root {
                    None
                } else {
                    parent_path.cloned()
                },
                label: node.label.clone(),
                shape: NodeShape::PartBlock,
                width,
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
                    is_def,
                    type_name,
                    type_def: node.type_def.clone(),
                    specializes,
                    multiplicity,
                    source_file,
                    origin: node.origin.clone(),
                    attributes,
                    short_id: None,
                    doc: None,
                    stereotype,
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
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };
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

    #[test]
    fn creating_part_def_displays_new_block_in_rover_diagram() {
        let mut app = SyssurApp::default();
        let part_def_item = crate::catalog::SYSML_V2_CATALOG
            .iter()
            .find(|item| item.id == "part_def")
            .expect("part_def catalog item");
        app.place_catalog_item(part_def_item, None, app.active_file.clone());

        let active_tid = app.active_view_tab.expect("active view tab");
        let active_tab = app.view_tabs.get(&active_tid).expect("active tab state");
        assert!(
            active_tab
                .layout
                .nodes
                .values()
                .any(|n| n.label.starts_with("NewPartDef")),
            "expected newly created part def block to appear in active diagram"
        );
    }

    #[test]
    fn creating_part_def_at_world_pos_places_block_at_drop_point() {
        let mut app = SyssurApp::default();
        let tab_id = app.active_view_tab.expect("active view tab");
        let part_def_item = crate::catalog::SYSML_V2_CATALOG
            .iter()
            .find(|item| item.id == "part_def")
            .expect("part_def catalog item");
        let drop_world = Point2 { x: 650.0, y: 420.0 };
        let ctx = egui::Context::default();
        app.handle_canvas_action(
            tab_id,
            CanvasAction::PlaceCatalogItem {
                item: part_def_item,
                container: None,
                file: app.active_file.clone(),
                world_pos: Some(drop_world),
            },
            &ctx,
        );

        let active_tab = app.view_tabs.get(&tab_id).expect("active tab state");
        let new_node = active_tab
            .layout
            .nodes
            .values()
            .find(|n| n.label.starts_with("NewPartDef"))
            .expect("newly placed part def node");
        let cx = new_node.rect.x + new_node.rect.w * 0.5;
        let cy = new_node.rect.y + new_node.rect.h * 0.5;
        assert!(
            (cx - drop_world.x).abs() < 1.0 && (cy - drop_world.y).abs() < 1.0,
            "expected node centered at ({}, {}), got ({cx}, {cy})",
            drop_world.x,
            drop_world.y
        );
    }

    #[test]
    fn block_auto_size_fit_grows_with_title_and_attributes() {
        let (w_short, h_short) = fit_part_block_size(
            "«part def»",
            "Wheel",
            None,
            None,
            None,
            &FeatureOrigin::Owned,
            &[],
            0,
        );
        let (w_long, _) = fit_part_block_size(
            "«part»",
            "powerSubsystemController",
            Some("RedundantPowerController"),
            None,
            Some(2),
            &FeatureOrigin::Owned,
            &[],
            0,
        );
        let attrs = vec![
            "mass : Real = 12.5".to_string(),
            "voltage : Real = 24.0".to_string(),
            "temperatureLimitCelsius : Real = 85.0".to_string(),
            "efficiency : Real = 0.94".to_string(),
        ];
        let (w_attrs, h_attrs) = fit_part_block_size(
            "«part def»",
            "Motor",
            None,
            None,
            None,
            &FeatureOrigin::Owned,
            &attrs,
            0,
        );

        assert!(w_short < 160.0 && h_short < 60.0, "compact block: ({w_short}, {h_short})");
        assert!(
            w_long > w_short + 150.0,
            "long title must expand width: short={w_short}, long={w_long}"
        );
        assert!(
            w_attrs > 280.0 && h_attrs > h_short + 40.0,
            "attributes must grow width and height: short=({w_short}, {h_short}), attrs=({w_attrs}, {h_attrs})"
        );
    }

    #[test]
    fn part_def_vs_usage_meta_matches_sysml_v2_notation() {
        let mut files = BTreeMap::new();
        files.insert(
            "notation.sysml".to_string(),
            r#"package P {
    abstract part def VehicleBase;
    part def Rover :> VehicleBase {
        part wheel : Wheel[4];
        attribute mass : Real;
    }
    part def Wheel;
    view V : GeneralView {
        expose P::**;
    }
}
"#
            .to_string(),
        );
        let parsed = SysmlEngine::parse_workspace(&files).graph;
        let view_id = ElementId::new("P::V");
        let (_, meta) = build_layout_input_for_view(
            &parsed,
            &ViewScope::SysmlView(view_id),
            ViewKind::General,
            LayoutDirection::TopToBottom,
        );

        let rover_meta = meta.get("P::Rover").expect("Rover definition NodeMeta");
        assert!(rover_meta.is_def);
        assert_eq!(rover_meta.stereotype, "«part def»");
        assert_eq!(rover_meta.specializes.as_deref(), Some("VehicleBase"));

        let base_meta = meta
            .get("P::VehicleBase")
            .expect("VehicleBase abstract definition NodeMeta");
        assert!(base_meta.is_def);
        assert_eq!(base_meta.stereotype, "«abstract part def»");

        let (_, wheel_meta) = meta
            .iter()
            .find(|(k, _)| k.contains("wheel"))
            .expect("wheel usage NodeMeta");
        assert!(!wheel_meta.is_def);
        assert_eq!(wheel_meta.stereotype, "«part»");
        assert_eq!(wheel_meta.multiplicity, Some(4));
    }

    #[test]
    fn selecting_connection_edge_populates_relationship_type_and_endpoints() {
        let mut app = SyssurApp::default();
        let (tab_id, edge_idx) = app
            .view_tabs
            .iter()
            .find_map(|(&tid, tab)| {
                tab.layout
                    .edges
                    .iter()
                    .enumerate()
                    .find(|(_, e)| !e.id.starts_with("spec#"))
                    .map(|(i, _)| (tid, i))
            })
            .expect("RoverSystem view with a routed relationship edge");

        let ctx = egui::Context::default();
        app.handle_canvas_action(tab_id, CanvasAction::SelectEdge { index: edge_idx }, &ctx);

        match &app.selected_target {
            SelectedTarget::Edge {
                source,
                target,
                kind,
                owner_scope,
                file,
                ..
            } => {
                assert!(!source.is_empty(), "expected non-empty source");
                assert!(!target.is_empty(), "expected non-empty target");
                assert!(!source.contains('#'), "source should be endpoint path, got {source}");
                assert!(owner_scope.is_some(), "expected owner scope on connection edge");
                assert!(!file.is_empty(), "expected source file");
                let (disp, kw, _) = crate::inspector::relationship_kind_info(*kind);
                assert!(!disp.is_empty() && !kw.is_empty());
            }
            other => panic!("expected SelectedTarget::Edge, got {other:?}"),
        }
    }

    #[test]
    fn resizing_block_updates_dimensions_and_pins_anchor_corner() {
        let mut app = SyssurApp::default();
        let tab_id = app.active_view_tab.expect("active view tab");
        let (node_key, orig_rect) = {
            let tab = app.view_tabs.get(&tab_id).expect("active tab");
            let (k, n) = tab.layout.nodes.iter().next().expect("at least one node");
            (k.clone(), n.rect)
        };
        let target_w = orig_rect.w + 90.0;
        let target_h = orig_rect.h + 55.0;
        let anchor_x = orig_rect.x + orig_rect.w;
        let anchor_y = orig_rect.y + orig_rect.h;
        let ctx = egui::Context::default();

        app.handle_canvas_action(
            tab_id,
            CanvasAction::ResizeNode {
                node_key: node_key.clone(),
                width: target_w,
                height: target_h,
                anchor_x,
                anchor_y,
                anchor_right: true,
                anchor_bottom: true,
            },
            &ctx,
        );

        let resized_rect = app.view_tabs[&tab_id].layout.nodes[&node_key].rect;
        assert!(
            (resized_rect.w - target_w).abs() < 1.0 && (resized_rect.h - target_h).abs() < 1.0,
            "expected resized ({target_w}, {target_h}), got ({}, {})",
            resized_rect.w,
            resized_rect.h
        );
        assert!(
            ((resized_rect.x + resized_rect.w) - anchor_x).abs() < 1.0
                && ((resized_rect.y + resized_rect.h) - anchor_y).abs() < 1.0,
            "expected bottom-right anchor ({anchor_x}, {anchor_y}) pinned, got ({}, {})",
            resized_rect.x + resized_rect.w,
            resized_rect.y + resized_rect.h
        );

        app.handle_canvas_action(tab_id, CanvasAction::AutoLayout, &ctx);
        assert!(app.view_tabs[&tab_id].node_size_overrides.is_empty());
    }

    #[test]
    fn deleting_selected_edge_removes_connection_from_model() {
        let mut app = SyssurApp::default();
        let initial_conns = app.graph.connections.len();
        assert!(initial_conns > 0, "default Rover preset has connections");

        let (tab_id, edge_idx) = app
            .view_tabs
            .iter()
            .find_map(|(&tid, tab)| {
                tab.layout
                    .edges
                    .iter()
                    .enumerate()
                    .find(|(_, e)| !e.id.starts_with("spec#"))
                    .map(|(i, _)| (tid, i))
            })
            .expect("routed non-spec connection edge");

        let ctx = egui::Context::default();
        app.handle_canvas_action(tab_id, CanvasAction::SelectEdge { index: edge_idx }, &ctx);
        assert!(
            matches!(
                &app.selected_target,
                SelectedTarget::Edge { span: Some(_), .. }
            ),
            "selected edge should carry source span for deletion"
        );

        app.delete_selected();
        assert_eq!(
            app.graph.connections.len(),
            initial_conns - 1,
            "deleting selected edge must remove one connection from model"
        );
        assert!(matches!(app.selected_target, SelectedTarget::None));
    }

    #[test]
    fn connecting_ports_creates_connection_in_model() {
        let mut app = SyssurApp::default();
        let initial_conns = app.graph.connections.len();

        let (src_ep, dst_ep) = app
            .view_tabs
            .values()
            .find_map(|tab| {
                let mut endpoints: Vec<PortEndpoint> = Vec::new();
                for node in tab.layout.nodes.values() {
                    if let Some(p) = node.ports.first() {
                        endpoints.push(PortEndpoint {
                            node_path: node.path.clone(),
                            node_label: node.label.clone(),
                            port_name: p.name.clone(),
                            port_type: p.type_label.clone(),
                            direction: p.direction,
                            is_conjugated: p.is_conjugated,
                            world_pos: egui::Pos2::new(p.anchor.x, p.anchor.y),
                        });
                    }
                }
                if endpoints.len() >= 2 {
                    Some((endpoints[0].clone(), endpoints[1].clone()))
                } else {
                    None
                }
            })
            .expect("at least two ported nodes in Rover preset");

        app.connect_ports(src_ep, dst_ep, Some("connection".to_string()));
        assert_eq!(
            app.graph.connections.len(),
            initial_conns + 1,
            "connecting two ports must add a connection to the parsed model"
        );
    }
}
