//! Scalable `egui::Painter` diagram canvas with `rstar::RTree` frustum culling,
//! 3-tier LOD rendering, full SysML v2 Clause 7 shapes & relationships,
//! free interactive node dragging with soft alignment guides & live orthogonal routing,
//! 4-way selection handles, searchable Clause 7 right-click context menu,
//! optional toggleable palette, and Explorer drag-to-expose support.

use crate::catalog::{
    catalog_item_row, catalog_relationships, has_items_for_context, show_create_submenus,
    show_optional_palette, CreateContext, SysmlCatalogItem,
};
use crate::i18n::{tr, Locale, TextKey};
use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};
use rstar::{RTree, RTreeObject, AABB};
use std::collections::BTreeMap;
use syssur_cmd::SyssurCommand;
use syssur_layout::{
    LayoutDirection, LayoutOutput, NodeShape, PortPlacement, PortSide, PositionedNode,
    PositionedPort, Rect2, RoutedEdge,
};
use syssur_model::{
    check_port_direction_and_type, ElementId, FeatureOrigin, InstancePath, PortCompatibility,
    PortDirection, RelationshipKind, ViewKind,
};

#[derive(Clone, Debug)]
pub struct NodeMeta {
    pub is_def: bool,
    pub type_name: Option<String>,
    pub type_def: Option<ElementId>,
    pub specializes: Option<String>,
    pub multiplicity: Option<u32>,
    pub source_file: String,
    pub origin: FeatureOrigin,
    pub attributes: Vec<String>,
    pub short_id: Option<String>,
    pub doc: Option<String>,
    pub stereotype: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortEndpoint {
    pub node_path: InstancePath,
    pub node_label: String,
    pub port_name: String,
    pub port_type: Option<String>,
    pub direction: PortDirection,
    pub is_conjugated: bool,
    pub world_pos: Pos2,
}

#[derive(Clone, Debug)]
pub struct WireDraft {
    pub source: PortEndpoint,
    pub current_world: Pos2,
    pub hovered_target: Option<(PortEndpoint, PortCompatibility)>,
    pub rel_keyword: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct NodeHandleDrag {
    pub source_path: InstancePath,
    pub source_label: String,
    pub source_shape: NodeShape,
    pub start_world: Pos2,
    pub current_world: Pos2,
    pub hovered_target: Option<(InstancePath, String, NodeShape, Pos2)>,
}

#[derive(Clone, Debug)]
pub struct NodeDragState {
    pub primary_key: String,
    pub dragged_keys: Vec<String>,
    pub press_screen: Pos2,
    pub last_world: Pos2,
    pub activated: bool,
    pub alignment_x: Option<f32>,
    pub alignment_y: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct PortDragState {
    pub node_key: String,
    pub port_name: String,
    pub node_rect: Rect2,
    pub press_screen: Pos2,
    pub activated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeCorner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

#[derive(Clone, Debug)]
pub struct NodeResizeState {
    pub node_key: String,
    pub corner: ResizeCorner,
    pub orig_rect: Rect2,
}

#[derive(Clone, Debug)]
pub struct PendingConnectChoice {
    pub source_path: InstancePath,
    pub source_label: String,
    pub target_label: String,
    pub screen_pos: Pos2,
    pub guard_draft: String,
}

#[derive(Clone, Debug)]
pub struct RelationshipPickState {
    pub rel_keyword: &'static str,
    pub rel_label: &'static str,
    pub source_path: InstancePath,
    pub source_label: String,
}

#[derive(Clone, Debug)]
pub enum CanvasAction {
    ConnectPorts {
        source: PortEndpoint,
        target: PortEndpoint,
        rel_keyword: Option<String>,
    },
    IncompatibleWireDrop {
        reason: String,
    },
    ConnectNodes {
        source_path: InstancePath,
        source_label: String,
        target_label: String,
        kind: RelationshipKind,
        guard: Option<String>,
    },
    CreateRelationship {
        rel_keyword: String,
        source_path: InstancePath,
        source_label: String,
        target_label: String,
    },
    PlaceCatalogItem {
        item: &'static SysmlCatalogItem,
        container: Option<InstancePath>,
        file: String,
        world_pos: Option<syssur_layout::Point2>,
    },
    MoveNodes {
        deltas: Vec<(String, f32, f32)>,
    },
    MovePort {
        node_key: String,
        port_name: String,
        placement: Option<PortPlacement>,
    },
    ResizeNode {
        node_key: String,
        width: f32,
        height: f32,
        anchor_x: f32,
        anchor_y: f32,
        anchor_right: bool,
        anchor_bottom: bool,
    },
    ResetNodeSize {
        node_key: String,
    },
    ApplyCommand(SyssurCommand),
    ExposeElement {
        target: ElementId,
    },
    SelectNode {
        path: InstancePath,
    },
    SelectPort {
        node_path: InstancePath,
        node_label: String,
        port_name: String,
    },
    SelectEdge {
        index: usize,
    },
    InspectAsRoot {
        target: ElementId,
    },
    RequestRename {
        element: ElementId,
        file: String,
    },
    DeleteElement {
        element: ElementId,
        file: String,
    },
    DeleteEdge {
        index: usize,
    },
    JumpToSource {
        file: String,
    },
    CopyPath {
        path: String,
    },
    AutoLayout,
    FitView,
}

#[derive(Clone)]
struct IndexedNode {
    key: String,
    envelope: AABB<[f32; 2]>,
}

impl RTreeObject for IndexedNode {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

#[derive(Clone)]
struct IndexedEdge {
    index: usize,
    envelope: AABB<[f32; 2]>,
}

impl RTreeObject for IndexedEdge {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderLod {
    Macro,
    Mid,
    Micro,
}

impl RenderLod {
    pub fn from_zoom(zoom: f32) -> Self {
        if zoom < 0.45 {
            Self::Macro
        } else if zoom < 0.85 {
            Self::Mid
        } else {
            Self::Micro
        }
    }
}

pub struct DiagramCanvas {
    pub pan: Vec2,
    pub zoom: f32,
    pub selected: Option<InstancePath>,
    pub selected_port: Option<(String, String)>,
    pub selected_edge: Option<usize>,
    context_node: Option<InstancePath>,
    context_port: Option<(String, String, String)>,
    context_edge: Option<usize>,
    context_world_pos: Option<Pos2>,
    wire_armed_port: Option<(PortEndpoint, Option<&'static str>)>,
    context_search: String,
    wire_draft: Option<WireDraft>,
    handle_drag: Option<NodeHandleDrag>,
    node_drag: Option<NodeDragState>,
    port_drag: Option<PortDragState>,
    node_resize: Option<NodeResizeState>,
    pending_choice: Option<PendingConnectChoice>,
    rel_pick: Option<RelationshipPickState>,
    node_tree: RTree<IndexedNode>,
    edge_tree: RTree<IndexedEdge>,
    pub last_visible_nodes: usize,
    pub last_visible_edges: usize,
}

impl Default for DiagramCanvas {
    fn default() -> Self {
        Self {
            pan: Vec2::new(60.0, 60.0),
            zoom: 1.0,
            selected: None,
            selected_port: None,
            selected_edge: None,
            context_node: None,
            context_port: None,
            context_edge: None,
            context_world_pos: None,
            wire_armed_port: None,
            context_search: String::new(),
            wire_draft: None,
            handle_drag: None,
            node_drag: None,
            port_drag: None,
            node_resize: None,
            pending_choice: None,
            rel_pick: None,
            node_tree: RTree::new(),
            edge_tree: RTree::new(),
            last_visible_nodes: 0,
            last_visible_edges: 0,
        }
    }
}

impl DiagramCanvas {
    pub fn cancel_interactions(&mut self) {
        self.wire_draft = None;
        self.handle_drag = None;
        self.node_drag = None;
        self.port_drag = None;
        self.node_resize = None;
        self.wire_armed_port = None;
        self.pending_choice = None;
        self.rel_pick = None;
    }

    pub fn arm_port_wire(&mut self, layout: &LayoutOutput, node_key: &str, port_name: &str) {
        if let Some(pnode) = layout.nodes.get(node_key) {
            if let Some(pp) = pnode.ports.iter().find(|p| p.name == port_name) {
                self.wire_armed_port = Some((
                    PortEndpoint {
                        node_path: pnode.path.clone(),
                        node_label: pnode.label.clone(),
                        port_name: pp.name.clone(),
                        port_type: pp.type_label.clone(),
                        direction: pp.direction,
                        is_conjugated: pp.is_conjugated,
                        world_pos: Pos2::new(pp.anchor.x, pp.anchor.y),
                    },
                    None,
                ));
            }
        }
    }

    fn port_wire_compatibility(
        rel_keyword: Option<&str>,
        source: &PortEndpoint,
        target: &PortEndpoint,
    ) -> PortCompatibility {
        if matches!(rel_keyword, Some("allocate" | "dependency" | "binding")) {
            return PortCompatibility::Compatible;
        }
        check_port_direction_and_type(
            source.port_type.as_deref(),
            source.direction.effective_direction(source.is_conjugated),
            target.port_type.as_deref(),
            target.direction.effective_direction(target.is_conjugated),
        )
    }

    pub fn rebuild_spatial_index(&mut self, layout: &LayoutOutput) {
        let nodes: Vec<IndexedNode> = layout
            .nodes
            .iter()
            .map(|(key, node)| IndexedNode {
                key: key.clone(),
                envelope: AABB::from_corners(
                    [node.rect.x - 18.0, node.rect.y - 18.0],
                    [node.rect.x + node.rect.w + 18.0, node.rect.y + node.rect.h + 18.0],
                ),
            })
            .collect();
        self.node_tree = RTree::bulk_load(nodes);

        let edges: Vec<IndexedEdge> = layout
            .edges
            .iter()
            .enumerate()
            .filter_map(|(index, edge)| {
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for p in &edge.waypoints {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
                if min_x > max_x {
                    return None;
                }
                Some(IndexedEdge {
                    index,
                    envelope: AABB::from_corners([min_x - 8.0, min_y - 8.0], [max_x + 8.0, max_y + 8.0]),
                })
            })
            .collect();
        self.edge_tree = RTree::bulk_load(edges);
    }

    pub fn fit_to_bounds(&mut self, layout: &LayoutOutput, viewport_size: Vec2) {
        if layout.bounds.w <= 1.0 || layout.bounds.h <= 1.0 {
            self.pan = Vec2::new(40.0, 40.0);
            self.zoom = 1.0;
            return;
        }
        let zx = (viewport_size.x * 0.88) / layout.bounds.w;
        let zy = (viewport_size.y * 0.88) / layout.bounds.h;
        self.zoom = zx.min(zy).clamp(0.2, 2.2);
        self.pan = Vec2::new(
            (viewport_size.x - layout.bounds.w * self.zoom) * 0.5 - layout.bounds.x * self.zoom,
            (viewport_size.y - layout.bounds.h * self.zoom) * 0.5 - layout.bounds.y * self.zoom,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        layout: &LayoutOutput,
        meta_by_key: &BTreeMap<String, NodeMeta>,
        package_names: &[String],
        view_kind: ViewKind,
        direction: LayoutDirection,
        palette_open: bool,
        palette_search: &mut String,
        active_tool: &mut Option<&'static SysmlCatalogItem>,
        active_file: &str,
        locale: Locale,
    ) -> Option<CanvasAction> {
        let mut action = None;

        if palette_open {
            ui.horizontal_top(|ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(26, 31, 39))
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        show_optional_palette(ui, palette_search, active_tool);
                    });

                action = self.paint_canvas_area(
                    ui,
                    layout,
                    meta_by_key,
                    package_names,
                    view_kind,
                    direction,
                    active_tool,
                    active_file,
                    locale,
                );
            });
        } else {
            action = self.paint_canvas_area(
                ui,
                layout,
                meta_by_key,
                package_names,
                view_kind,
                direction,
                active_tool,
                active_file,
                locale,
            );
        }

        if let Some(mut choice) = self.pending_choice.take() {
            let mut keep = true;
            let area_resp = egui::Area::new(egui::Id::unique("connector_disambiguation_popup"))
                .order(egui::Order::Foreground)
                .constrain(true)
                .fixed_pos(choice.screen_pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} -> {}",
                                choice.source_label, choice.target_label
                            ))
                            .strong(),
                        );
                        ui.separator();
                        for (lbl, kw) in [
                            ("Connection (connect)", "connection"),
                            ("Interface", "interface"),
                            ("Flow", "flow"),
                            ("Succession", "succession"),
                            ("State Transition", "transition"),
                            ("Satisfy Requirement", "satisfy"),
                            ("Verify Requirement", "verify"),
                            ("Allocate", "allocate"),
                            ("Dependency", "dependency"),
                            ("Specialize (:>)", "specialization"),
                        ] {
                            if ui.button(lbl).clicked() {
                                action = Some(CanvasAction::CreateRelationship {
                                    rel_keyword: kw.to_string(),
                                    source_path: choice.source_path.clone(),
                                    source_label: choice.source_label.clone(),
                                    target_label: choice.target_label.clone(),
                                });
                                keep = false;
                            }
                        }
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut choice.guard_draft)
                                    .desired_width(110.0)
                                    .hint_text("guard expr"),
                            );
                            if ui.button("Guarded").clicked() && !choice.guard_draft.trim().is_empty()
                            {
                                action = Some(CanvasAction::ConnectNodes {
                                    source_path: choice.source_path.clone(),
                                    source_label: choice.source_label.clone(),
                                    target_label: choice.target_label.clone(),
                                    kind: RelationshipKind::Succession,
                                    guard: Some(choice.guard_draft.trim().to_string()),
                                });
                                keep = false;
                            }
                        });
                        if ui.button(tr(locale, TextKey::Cancel)).clicked() {
                            keep = false;
                        }
                    });
                });
            if ui.input(|i| i.key_pressed(egui::Key::Escape))
                || area_resp.response.clicked_elsewhere()
            {
                keep = false;
            }
            if keep {
                self.pending_choice = Some(choice);
            }
        }

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_canvas_area(
        &mut self,
        ui: &mut Ui,
        layout: &LayoutOutput,
        meta_by_key: &BTreeMap<String, NodeMeta>,
        package_names: &[String],
        view_kind: ViewKind,
        direction: LayoutDirection,
        active_tool: &mut Option<&'static SysmlCatalogItem>,
        active_file: &str,
        locale: Locale,
    ) -> Option<CanvasAction> {
        let mut action = None;
        let avail = ui.available_rect_before_wrap();
        let (response, painter) = ui.allocate_painter(avail.size(), Sense::click_and_drag());

        painter.rect_filled(avail, 0.0, Color32::from_rgb(20, 24, 30));

        // Explorer drag-to-expose drop detection
        if egui::DragAndDrop::payload::<ElementId>(ui.ctx()).is_some() && response.hovered() {
            painter.rect_stroke(
                avail.shrink(3.0),
                4.0,
                Stroke::new(2.0, Color32::from_rgb(96, 175, 255)),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(payload) = egui::DragAndDrop::take_payload::<ElementId>(ui.ctx()) {
                    action = Some(CanvasAction::ExposeElement {
                        target: (*payload).clone(),
                    });
                }
            }
        }

        let origin = avail.min;
        let world_tl = self.screen_to_world(avail.min, origin);
        let world_br = self.screen_to_world(avail.max, origin);
        let view_aabb = AABB::from_corners([world_tl.x, world_tl.y], [world_br.x, world_br.y]);
        let lod = RenderLod::from_zoom(self.zoom);

        let mut visible_nodes: Vec<&PositionedNode> = self
            .node_tree
            .locate_in_envelope_intersecting(view_aabb)
            .filter_map(|in_| layout.nodes.get(&in_.key))
            .collect();
        visible_nodes.sort_by(|a, b| {
            (b.rect.w * b.rect.h)
                .total_cmp(&(a.rect.w * a.rect.h))
                .then_with(|| a.path.canonical_key().cmp(&b.path.canonical_key()))
        });
        self.last_visible_nodes = visible_nodes.len();

        // Palette item drag-and-drop onto canvas
        if egui::DragAndDrop::payload::<&'static SysmlCatalogItem>(ui.ctx()).is_some()
            && response.hovered()
        {
            painter.rect_stroke(
                avail.shrink(3.0),
                4.0,
                Stroke::new(2.0, Color32::from_rgb(100, 215, 165)),
                StrokeKind::Inside,
            );
            if ui.input(|i| i.pointer.any_released()) {
                if let Some(payload) =
                    egui::DragAndDrop::take_payload::<&'static SysmlCatalogItem>(ui.ctx())
                {
                    let item = *payload;
                    if item.is_relationship {
                        *active_tool = Some(item);
                    } else {
                        let drop_pos = ui.input(|i| i.pointer.interact_pos());
                        let world_pos = drop_pos.map(|p| {
                            let w = self.screen_to_world(p, origin);
                            syssur_layout::Point2 { x: w.x, y: w.y }
                        });
                        let hit_container = drop_pos.and_then(|p| {
                            Self::hit_test_node_static(
                                self.pan,
                                self.zoom,
                                &visible_nodes,
                                p,
                                origin,
                            )
                            .map(|n| n.path.clone())
                        });
                        let file = hit_container
                            .as_ref()
                            .and_then(|p| meta_by_key.get(&p.canonical_key()))
                            .map(|m| m.source_file.clone())
                            .unwrap_or_else(|| active_file.to_string());
                        action = Some(CanvasAction::PlaceCatalogItem {
                            item,
                            container: hit_container,
                            file,
                            world_pos,
                        });
                    }
                }
            }
        }

        // Drag start priority: 0) Corner Resize -> 1) Port (Shift+Drag or relationship tool -> Wire; plain drag -> Reposition) -> 2) 4-way handle -> 3) Free Node Move -> 4) Background pan
        let shift_held = ui.input(|i| i.modifiers.shift);
        let rel_tool_kw = active_tool.filter(|t| t.is_relationship).map(|t| t.keyword);
        if self.node_resize.is_none()
            && self.wire_draft.is_none()
            && self.handle_drag.is_none()
            && self.node_drag.is_none()
            && self.port_drag.is_none()
            && response.drag_started_by(egui::PointerButton::Primary)
        {
            if let Some(press_pos) = ui.input(|i| i.pointer.press_origin()) {
                if let Some((rnode, corner)) =
                    self.hit_test_resize_handle(layout, press_pos, origin)
                {
                    self.node_resize = Some(NodeResizeState {
                        node_key: rnode.path.canonical_key(),
                        corner,
                        orig_rect: rnode.rect,
                    });
                } else if let Some(src_port) = self.hit_test_port(&visible_nodes, press_pos, origin) {
                    if self.wire_armed_port.is_some() || self.rel_pick.is_some() {
                        // Let primary click release complete 2-click port/relationship wiring.
                    } else if shift_held || rel_tool_kw.is_some() {
                        let world_pos = self.screen_to_world(press_pos, origin);
                        self.wire_draft = Some(WireDraft {
                            source: src_port,
                            current_world: world_pos,
                            hovered_target: None,
                            rel_keyword: rel_tool_kw,
                        });
                    } else {
                        let node_key = src_port.node_path.canonical_key();
                        if let Some(pnode) = layout.nodes.get(&node_key) {
                            self.selected = None;
                            self.selected_port = Some((node_key.clone(), src_port.port_name.clone()));
                            self.selected_edge = None;
                            self.port_drag = Some(PortDragState {
                                node_key,
                                port_name: src_port.port_name.clone(),
                                node_rect: pnode.rect,
                                press_screen: press_pos,
                                activated: false,
                            });
                            action = Some(CanvasAction::SelectPort {
                                node_path: src_port.node_path,
                                node_label: src_port.node_label,
                                port_name: src_port.port_name,
                            });
                        }
                    }
                } else if let Some(handle_src) =
                    self.hit_test_selection_handle(layout, press_pos, origin)
                {
                    let world_pos = self.screen_to_world(press_pos, origin);
                    self.handle_drag = Some(NodeHandleDrag {
                        source_path: handle_src.path.clone(),
                        source_label: handle_src.label.clone(),
                        source_shape: handle_src.shape,
                        start_world: world_pos,
                        current_world: world_pos,
                        hovered_target: None,
                    });
                } else if active_tool.is_none()
                    && self.rel_pick.is_none()
                    && self.wire_armed_port.is_none()
                {
                    if let Some(hit_node) = Self::hit_test_node_static(
                        self.pan,
                        self.zoom,
                        &visible_nodes,
                        press_pos,
                        origin,
                    ) {
                        let primary_key = hit_node.path.canonical_key();
                        let prefix = format!("{primary_key}/");
                        let mut dragged_keys = vec![primary_key.clone()];
                        for k in layout.nodes.keys() {
                            if k.starts_with(&prefix) {
                                dragged_keys.push(k.clone());
                            }
                        }
                        self.selected = Some(hit_node.path.clone());
                        self.selected_edge = None;
                        self.node_drag = Some(NodeDragState {
                            primary_key,
                            dragged_keys,
                            press_screen: press_pos,
                            last_world: self.screen_to_world(press_pos, origin),
                            activated: false,
                            alignment_x: None,
                            alignment_y: None,
                        });
                        action = Some(CanvasAction::SelectNode {
                            path: hit_node.path.clone(),
                        });
                    }
                }
            }
        }

        let pointer_interact = ui.input(|i| i.pointer.interact_pos());
        let pointer_world = pointer_interact.map(|p| self.screen_to_world(p, origin));
        let pan = self.pan;
        let zoom = self.zoom;

        if let Some(ref rstate) = self.node_resize {
            let icon = match rstate.corner {
                ResizeCorner::TopLeft | ResizeCorner::BottomRight => egui::CursorIcon::ResizeNwSe,
                ResizeCorner::TopRight | ResizeCorner::BottomLeft => egui::CursorIcon::ResizeNeSw,
            };
            ui.ctx().set_cursor_icon(icon);
        } else if self.port_drag.as_ref().is_some_and(|d| d.activated) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        } else if response.hovered() {
            if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
                if let Some((_, corner)) = self.hit_test_resize_handle(layout, ptr, origin) {
                    let icon = match corner {
                        ResizeCorner::TopLeft | ResizeCorner::BottomRight => {
                            egui::CursorIcon::ResizeNwSe
                        }
                        ResizeCorner::TopRight | ResizeCorner::BottomLeft => {
                            egui::CursorIcon::ResizeNeSw
                        }
                    };
                    ui.ctx().set_cursor_icon(icon);
                } else if Self::hit_test_port_static(pan, zoom, &visible_nodes, ptr, origin)
                    .is_some()
                {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }

        const DRAG_DEADZONE_PX: f32 = 4.0;
        if let Some(ref rstate) = self.node_resize.clone() {
            if let Some(world_ptr) = pointer_world {
                let o = rstate.orig_rect;
                let right = o.x + o.w;
                let bottom = o.y + o.h;
                let (raw_w, raw_h, anchor_x, anchor_y, anchor_right, anchor_bottom) =
                    match rstate.corner {
                        ResizeCorner::BottomRight => {
                            (world_ptr.x - o.x, world_ptr.y - o.y, o.x, o.y, false, false)
                        }
                        ResizeCorner::BottomLeft => {
                            (right - world_ptr.x, world_ptr.y - o.y, right, o.y, true, false)
                        }
                        ResizeCorner::TopRight => {
                            (world_ptr.x - o.x, bottom - world_ptr.y, o.x, bottom, false, true)
                        }
                        ResizeCorner::TopLeft => (
                            right - world_ptr.x,
                            bottom - world_ptr.y,
                            right,
                            bottom,
                            true,
                            true,
                        ),
                    };
                action = Some(CanvasAction::ResizeNode {
                    node_key: rstate.node_key.clone(),
                    width: raw_w.clamp(80.0, 900.0),
                    height: raw_h.clamp(36.0, 700.0),
                    anchor_x,
                    anchor_y,
                    anchor_right,
                    anchor_bottom,
                });
            }
            if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                self.node_resize = None;
            }
        } else if let Some(ref mut pdrag) = self.port_drag {
            if let Some(screen_ptr) = pointer_interact {
                if !pdrag.activated && screen_ptr.distance(pdrag.press_screen) >= DRAG_DEADZONE_PX {
                    pdrag.activated = true;
                }
                if pdrag.activated {
                    if let Some(world_ptr) = pointer_world {
                        let placement = project_point_to_border(world_ptr, pdrag.node_rect);
                        action = Some(CanvasAction::MovePort {
                            node_key: pdrag.node_key.clone(),
                            port_name: pdrag.port_name.clone(),
                            placement: Some(placement),
                        });
                    }
                }
            }
            if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                self.port_drag = None;
            }
        } else if let Some(ref mut draft) = self.wire_draft {
            if let (Some(pointer), Some(world_ptr)) = (pointer_interact, pointer_world) {
                draft.current_world = world_ptr;
                draft.hovered_target =
                    Self::hit_test_port_static(pan, zoom, &visible_nodes, pointer, origin)
                        .filter(|target| {
                            !(target.node_path == draft.source.node_path
                                && target.port_name == draft.source.port_name)
                        })
                        .map(|target| {
                            let compat = Self::port_wire_compatibility(
                                draft.rel_keyword,
                                &draft.source,
                                &target,
                            );
                            (target, compat)
                        });
            }
            if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                if let Some(done) = self.wire_draft.take() {
                    if let Some((target, compat)) = done.hovered_target {
                        match compat {
                            PortCompatibility::Compatible => {
                                action = Some(CanvasAction::ConnectPorts {
                                    source: done.source,
                                    target,
                                    rel_keyword: done.rel_keyword.map(str::to_string),
                                });
                                if done.rel_keyword.is_some() {
                                    *active_tool = None;
                                }
                            }
                            PortCompatibility::DirectionMismatch {
                                source_dir,
                                target_dir,
                            } => {
                                action = Some(CanvasAction::IncompatibleWireDrop {
                                    reason: format!(
                                        "Direction mismatch: {source_dir:?} -> {target_dir:?} (conjugate consumer port with ~)"
                                    ),
                                });
                            }
                            PortCompatibility::TypeMismatch {
                                source_type,
                                target_type,
                            } => {
                                action = Some(CanvasAction::IncompatibleWireDrop {
                                    reason: format!(
                                        "PortDef type mismatch: {source_type} != {target_type}"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        } else if let Some(ref mut hdrag) = self.handle_drag {
            if let (Some(pointer), Some(world_ptr)) = (pointer_interact, pointer_world) {
                hdrag.current_world = world_ptr;
                hdrag.hovered_target =
                    Self::hit_test_node_static(pan, zoom, &visible_nodes, pointer, origin)
                        .filter(|n| n.path != hdrag.source_path)
                        .map(|n| {
                            (
                                n.path.clone(),
                                n.label.clone(),
                                n.shape,
                                Pos2::new(n.rect.x + n.rect.w * 0.5, n.rect.y + n.rect.h * 0.5),
                            )
                        });
            }
            if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                if let Some(done) = self.handle_drag.take() {
                    if let Some((_t_path, t_label, t_shape, _)) = done.hovered_target {
                        if let Some(auto_kind) =
                            infer_unambiguous_link(done.source_shape, t_shape, view_kind)
                        {
                            action = Some(CanvasAction::ConnectNodes {
                                source_path: done.source_path,
                                source_label: done.source_label,
                                target_label: t_label,
                                kind: auto_kind,
                                guard: None,
                            });
                        } else {
                            let drop_screen = pointer_interact.unwrap_or(avail.center());
                            self.pending_choice = Some(PendingConnectChoice {
                                source_path: done.source_path,
                                source_label: done.source_label,
                                target_label: t_label,
                                screen_pos: drop_screen,
                                guard_draft: String::new(),
                            });
                        }
                    }
                }
            }
        } else if let Some(ref mut ndrag) = self.node_drag {
            if !ndrag.activated {
                if let Some(screen_ptr) = pointer_interact {
                    if screen_ptr.distance(ndrag.press_screen) >= DRAG_DEADZONE_PX {
                        ndrag.activated = true;
                    }
                }
            }
            if ndrag.activated {
                if let Some(world_ptr) = pointer_world {
                    let mut dx = world_ptr.x - ndrag.last_world.x;
                    let mut dy = world_ptr.y - ndrag.last_world.y;
                    ndrag.alignment_x = None;
                    ndrag.alignment_y = None;

                    if dx.abs() > 0.05 || dy.abs() > 0.05 {
                        if let Some(primary) = layout.nodes.get(&ndrag.primary_key) {
                            let cand_cx = primary.rect.x + primary.rect.w * 0.5 + dx;
                            let cand_cy = primary.rect.y + primary.rect.h * 0.5 + dy;
                            for (okey, other) in &layout.nodes {
                                if ndrag.dragged_keys.contains(okey) {
                                    continue;
                                }
                                let ocx = other.rect.x + other.rect.w * 0.5;
                                let ocy = other.rect.y + other.rect.h * 0.5;
                                if (cand_cx - ocx).abs() < 6.0 && ndrag.alignment_x.is_none() {
                                    dx += ocx - cand_cx;
                                    ndrag.alignment_x = Some(ocx);
                                }
                                if (cand_cy - ocy).abs() < 6.0 && ndrag.alignment_y.is_none() {
                                    dy += ocy - cand_cy;
                                    ndrag.alignment_y = Some(ocy);
                                }
                            }
                        }
                        ndrag.last_world =
                            Pos2::new(ndrag.last_world.x + dx, ndrag.last_world.y + dy);
                        let deltas = ndrag
                            .dragged_keys
                            .iter()
                            .map(|k| (k.clone(), dx, dy))
                            .collect();
                        action = Some(CanvasAction::MoveNodes { deltas });
                    }
                }
            }
            if response.drag_stopped() || ui.input(|i| i.pointer.any_released()) {
                self.node_drag = None;
            }
        } else if response.dragged_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Middle)
        {
            self.pan += response.drag_delta();
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.1 {
                let factor = (1.0 + scroll * 0.0015).clamp(0.8, 1.25);
                if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                    let world_before = self.screen_to_world(pointer, avail.min);
                    self.zoom = (self.zoom * factor).clamp(0.15, 3.5);
                    let screen_after = self.world_to_screen(world_before, avail.min);
                    self.pan += pointer - screen_after;
                } else {
                    self.zoom = (self.zoom * factor).clamp(0.15, 3.5);
                }
            }
        }

        let clipped = painter.with_clip_rect(avail);

        // Alignment guide lines while moving a node freely
        if let Some(ref ndrag) = self.node_drag {
            let guide_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(110, 210, 255, 160));
            if let Some(ax) = ndrag.alignment_x {
                let sx = self.world_to_screen(Pos2::new(ax, 0.0), origin).x;
                clipped.line_segment(
                    [Pos2::new(sx, avail.top()), Pos2::new(sx, avail.bottom())],
                    guide_stroke,
                );
            }
            if let Some(ay) = ndrag.alignment_y {
                let sy = self.world_to_screen(Pos2::new(0.0, ay), origin).y;
                clipped.line_segment(
                    [Pos2::new(avail.left(), sy), Pos2::new(avail.right(), sy)],
                    guide_stroke,
                );
            }
        }

        let click_pos = if response.clicked() {
            response.interact_pointer_pos()
        } else {
            None
        };

        // 2-click relationship wiring, 2-click port wiring, or active catalog tool placement
        if let Some(cp) = click_pos {
            if let Some((src_ep, rel_kw)) = self.wire_armed_port.take() {
                if let Some(target) =
                    Self::hit_test_port_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                {
                    if !(target.node_path == src_ep.node_path
                        && target.port_name == src_ep.port_name)
                    {
                        match Self::port_wire_compatibility(rel_kw, &src_ep, &target) {
                            PortCompatibility::Compatible => {
                                action = Some(CanvasAction::ConnectPorts {
                                    source: src_ep,
                                    target,
                                    rel_keyword: rel_kw.map(str::to_string),
                                });
                                if rel_kw.is_some() {
                                    *active_tool = None;
                                }
                            }
                            PortCompatibility::DirectionMismatch {
                                source_dir,
                                target_dir,
                            } => {
                                action = Some(CanvasAction::IncompatibleWireDrop {
                                    reason: format!(
                                        "Direction mismatch: {source_dir:?} -> {target_dir:?} (conjugate consumer port with ~)"
                                    ),
                                });
                            }
                            PortCompatibility::TypeMismatch {
                                source_type,
                                target_type,
                            } => {
                                action = Some(CanvasAction::IncompatibleWireDrop {
                                    reason: format!(
                                        "PortDef type mismatch: {source_type} != {target_type}"
                                    ),
                                });
                            }
                        }
                    }
                }
            } else if let Some(ref pick) = self.rel_pick.clone() {
                if let Some(target_port) =
                    Self::hit_test_port_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                {
                    if target_port.node_path != pick.source_path {
                        action = Some(CanvasAction::CreateRelationship {
                            rel_keyword: pick.rel_keyword.to_string(),
                            source_path: pick.source_path.clone(),
                            source_label: pick.source_label.clone(),
                            target_label: format!(
                                "{}.{}",
                                target_port.node_label, target_port.port_name
                            ),
                        });
                        self.rel_pick = None;
                        *active_tool = None;
                    }
                } else if let Some(target_node) =
                    Self::hit_test_node_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                {
                    if target_node.path != pick.source_path {
                        action = Some(CanvasAction::CreateRelationship {
                            rel_keyword: pick.rel_keyword.to_string(),
                            source_path: pick.source_path.clone(),
                            source_label: pick.source_label.clone(),
                            target_label: target_node.label.clone(),
                        });
                        self.rel_pick = None;
                        *active_tool = None;
                    }
                } else {
                    self.rel_pick = None;
                }
            } else if let Some(tool) = *active_tool {
                if tool.is_relationship {
                    if let Some(src_port) =
                        Self::hit_test_port_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                    {
                        self.wire_armed_port = Some((src_port, Some(tool.keyword)));
                    } else if let Some(src_node) =
                        Self::hit_test_node_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                    {
                        self.rel_pick = Some(RelationshipPickState {
                            rel_keyword: tool.keyword,
                            rel_label: tool.label,
                            source_path: src_node.path.clone(),
                            source_label: src_node.label.clone(),
                        });
                    }
                } else {
                    let hit_container =
                        Self::hit_test_node_static(self.pan, self.zoom, &visible_nodes, cp, origin)
                            .map(|n| n.path.clone());
                    let file = hit_container
                        .as_ref()
                        .and_then(|p| meta_by_key.get(&p.canonical_key()))
                        .map(|m| m.source_file.clone())
                        .unwrap_or_else(|| active_file.to_string());
                    let world_pos = {
                        let w = self.screen_to_world(cp, origin);
                        Some(syssur_layout::Point2 { x: w.x, y: w.y })
                    };
                    action = Some(CanvasAction::PlaceCatalogItem {
                        item: tool,
                        container: hit_container,
                        file,
                        world_pos,
                    });
                    *active_tool = None;
                }
            }
        }

        if response.secondary_clicked() {
            if let Some(rc_pos) = response.interact_pointer_pos() {
                self.context_world_pos = Some(self.screen_to_world(rc_pos, origin));
                self.context_port = None;
                self.context_node = None;
                self.context_edge = None;
                if let Some(port_ep) = self.hit_test_port(&visible_nodes, rc_pos, origin) {
                    let node_key = port_ep.node_path.canonical_key();
                    let file = meta_by_key
                        .get(&node_key)
                        .map(|m| m.source_file.clone())
                        .unwrap_or_else(|| active_file.to_string());
                    self.context_port = Some((node_key.clone(), port_ep.port_name.clone(), file));
                    self.selected = None;
                    self.selected_port = Some((node_key, port_ep.port_name));
                    self.selected_edge = None;
                } else {
                    let mut hit_leaf_rc = None;
                    let mut hit_container_rc = None;
                    for &node in visible_nodes.iter().rev() {
                        let screen_rect = Rect::from_min_size(
                            self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
                            Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
                        );
                        if screen_rect.contains(rc_pos) {
                            let is_container = visible_nodes.iter().any(|other| {
                                other.path != node.path
                                    && other.path.parent().as_ref() == Some(&node.path)
                            });
                            if is_container {
                                if hit_container_rc.is_none() {
                                    hit_container_rc = Some(node.path.clone());
                                }
                            } else {
                                hit_leaf_rc = Some(node.path.clone());
                                break;
                            }
                        }
                    }
                    if let Some(path) = hit_leaf_rc {
                        self.context_node = Some(path.clone());
                        self.selected = Some(path);
                        self.selected_port = None;
                        self.selected_edge = None;
                    } else if let Some(edge_idx) = self
                        .edge_tree
                        .locate_in_envelope_intersecting(view_aabb)
                        .find_map(|ie| {
                            layout
                                .edges
                                .get(ie.index)
                                .filter(|e| self.hit_test_edge(e, rc_pos, origin))
                                .map(|_| ie.index)
                        })
                    {
                        self.context_edge = Some(edge_idx);
                        self.selected = None;
                        self.selected_port = None;
                        self.selected_edge = Some(edge_idx);
                        action = Some(CanvasAction::SelectEdge { index: edge_idx });
                    } else if let Some(path) = hit_container_rc {
                        self.context_node = Some(path.clone());
                        self.selected = Some(path);
                        self.selected_port = None;
                        self.selected_edge = None;
                    }
                }
            }
        }

        // Partition visible nodes into compound containers vs leaf blocks so edges NEVER draw over leaf blocks.
        let mut container_nodes: Vec<&PositionedNode> = Vec::new();
        let mut leaf_nodes: Vec<&PositionedNode> = Vec::new();
        let mut hit_port_this_click = click_pos.and_then(|cp| {
            if action.is_none() && active_tool.is_none() && self.rel_pick.is_none() {
                self.hit_test_port(&visible_nodes, cp, origin)
            } else {
                None
            }
        });
        let mut hit_leaf_this_click = None;
        let mut hit_container_this_click = None;
        for &node in &visible_nodes {
            let is_container = visible_nodes.iter().any(|other| {
                other.path != node.path && other.path.parent().as_ref() == Some(&node.path)
            });
            let screen_rect = Rect::from_min_size(
                self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
                Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
            );
            if let Some(cp) = click_pos {
                if action.is_none()
                    && active_tool.is_none()
                    && self.rel_pick.is_none()
                    && hit_port_this_click.is_none()
                    && screen_rect.contains(cp)
                {
                    if is_container {
                        hit_container_this_click = Some(node.path.clone());
                    } else {
                        hit_leaf_this_click = Some(node.path.clone());
                    }
                }
            }
            if is_container {
                container_nodes.push(node);
            } else {
                leaf_nodes.push(node);
            }
        }

        // Layer 1: Compound container bodies (deferring ports until after edges)
        for node in &container_nodes {
            let key = node.path.canonical_key();
            let meta = meta_by_key.get(&key);
            let screen_rect = Rect::from_min_size(
                self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
                Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
            );
            let is_selected = self.selected.as_ref() == Some(&node.path);
            self.paint_node(
                &clipped,
                node,
                meta,
                screen_rect,
                origin,
                lod,
                is_selected,
                true,
                direction,
                false,
            );
        }

        // Layer 2: Frustum-culled edges (lines + arrowheads, underneath leaf blocks)
        let visible_edges: Vec<(usize, &RoutedEdge)> = self
            .edge_tree
            .locate_in_envelope_intersecting(view_aabb)
            .filter_map(|ie| layout.edges.get(ie.index).map(|e| (ie.index, e)))
            .collect();
        self.last_visible_edges = visible_edges.len();

        let mut hit_edge_this_click = None;
        for &(idx, edge) in &visible_edges {
            if let Some(cp) = click_pos {
                if action.is_none()
                    && active_tool.is_none()
                    && self.rel_pick.is_none()
                    && hit_port_this_click.is_none()
                    && hit_leaf_this_click.is_none()
                    && self.hit_test_edge(edge, cp, origin)
                {
                    hit_edge_this_click = Some(idx);
                }
            }
            let is_selected = self.selected_edge == Some(idx);
            self.paint_edge_geometry(&clipped, edge, origin, is_selected);
        }

        // Layer 3: Opaque leaf blocks + their border ports ABOVE connectors
        for node in &leaf_nodes {
            let key = node.path.canonical_key();
            let meta = meta_by_key.get(&key);
            let screen_rect = Rect::from_min_size(
                self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
                Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
            );
            let is_selected = self.selected.as_ref() == Some(&node.path);
            self.paint_node(
                &clipped,
                node,
                meta,
                screen_rect,
                origin,
                lod,
                is_selected,
                false,
                direction,
                true,
            );
        }

        // Layer 4: Compound container border ports above edge endpoints
        if lod == RenderLod::Micro {
            for node in &container_nodes {
                let nkey = node.path.canonical_key();
                for port in &node.ports {
                    self.paint_port(&clipped, &nkey, port, origin);
                }
            }
        }

        // Layer 5: Edge guard badges & relationship labels on top
        if lod == RenderLod::Micro {
            for &(idx, edge) in &visible_edges {
                let is_selected = self.selected_edge == Some(idx);
                self.paint_edge_label(&clipped, edge, origin, is_selected);
            }
        }

        if let Some(port_ep) = hit_port_this_click.take() {
            let node_key = port_ep.node_path.canonical_key();
            self.selected = None;
            self.selected_port = Some((node_key, port_ep.port_name.clone()));
            self.selected_edge = None;
            if action.is_none() {
                action = Some(CanvasAction::SelectPort {
                    node_path: port_ep.node_path,
                    node_label: port_ep.node_label,
                    port_name: port_ep.port_name,
                });
            }
        } else if let Some(path) = hit_leaf_this_click {
            self.selected = Some(path.clone());
            self.selected_port = None;
            self.selected_edge = None;
            if action.is_none() {
                action = Some(CanvasAction::SelectNode { path });
            }
        } else if let Some(edge_idx) = hit_edge_this_click {
            self.selected = None;
            self.selected_port = None;
            self.selected_edge = Some(edge_idx);
            if action.is_none() {
                action = Some(CanvasAction::SelectEdge { index: edge_idx });
            }
        } else if let Some(path) = hit_container_this_click {
            self.selected = Some(path.clone());
            self.selected_port = None;
            self.selected_edge = None;
            if action.is_none() {
                action = Some(CanvasAction::SelectNode { path });
            }
        } else if click_pos.is_some()
            && action.is_none()
            && active_tool.is_none()
            && self.rel_pick.is_none()
        {
            self.selected = None;
            self.selected_port = None;
            self.selected_edge = None;
        }

        if let Some(ref sel_path) = self.selected {
            if let Some(sel_node) = layout.nodes.get(&sel_path.canonical_key()) {
                self.paint_selection_handles(&clipped, sel_node, origin);
            }
        }

        if let Some(ref draft) = self.wire_draft {
            self.paint_wire_draft(&clipped, draft, origin);
        } else if let Some((ref src_ep, rel_kw)) = self.wire_armed_port {
            if let (Some(pointer), Some(world_ptr)) = (pointer_interact, pointer_world) {
                let hovered = Self::hit_test_port_static(pan, zoom, &visible_nodes, pointer, origin)
                    .filter(|t| !(t.node_path == src_ep.node_path && t.port_name == src_ep.port_name))
                    .map(|target| {
                        let compat = Self::port_wire_compatibility(rel_kw, src_ep, &target);
                        (target, compat)
                    });
                let preview = WireDraft {
                    source: src_ep.clone(),
                    current_world: world_ptr,
                    hovered_target: hovered,
                    rel_keyword: rel_kw,
                };
                self.paint_wire_draft(&clipped, &preview, origin);
            }
        }
        if let Some(ref hdrag) = self.handle_drag {
            self.paint_handle_drag(&clipped, hdrag, origin);
        }
        if let Some(ref pick) = self.rel_pick {
            if let (Some(src_node), Some(ptr)) = (
                layout.nodes.get(&pick.source_path.canonical_key()),
                ui.input(|i| i.pointer.hover_pos()),
            ) {
                let start = self.world_to_screen(
                    Pos2::new(
                        src_node.rect.x + src_node.rect.w * 0.5,
                        src_node.rect.y + src_node.rect.h * 0.5,
                    ),
                    origin,
                );
                let col = Color32::from_rgb(130, 215, 255);
                paint_dashed_line(&clipped, start, ptr, Stroke::new(2.0, col));
                paint_arrowhead(&clipped, start, ptr, col);
                clipped.text(
                    ptr + Vec2::new(10.0, -10.0),
                    Align2::LEFT_BOTTOM,
                    format!("Click target for {} (Esc to cancel)", pick.rel_label),
                    FontId::monospace(11.0),
                    col,
                );
            }
        }

        // Clean, non-convoluted nested Clause 7 right-click contextual menu (Port vs Node vs Canvas)
        egui::Popup::context_menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
            if let Some((node_key, port_name, port_file)) = self.context_port.clone() {
                ui.set_min_width(220.0);
                ui.label(RichText::new(format!("Port: {port_name}")).strong());
                ui.separator();

                let cur_offset = layout
                    .nodes
                    .get(&node_key)
                    .and_then(|n| n.ports.iter().find(|p| p.name == port_name))
                    .map(|p| p.offset)
                    .unwrap_or(0.5);
                for (lbl, side) in [
                    ("Move to Top Border", PortSide::Top),
                    ("Move to Right Border", PortSide::Right),
                    ("Move to Bottom Border", PortSide::Bottom),
                    ("Move to Left Border", PortSide::Left),
                ] {
                    if ui.button(lbl).clicked() {
                        action = Some(CanvasAction::MovePort {
                            node_key: node_key.clone(),
                            port_name: port_name.clone(),
                            placement: Some(PortPlacement {
                                side,
                                offset: cur_offset,
                            }),
                        });
                        ui.close();
                    }
                }
                if ui.button("Wire / Connect from Port...").clicked() {
                    if let Some(pnode) = layout.nodes.get(&node_key) {
                        if let Some(pp) = pnode.ports.iter().find(|p| p.name == port_name) {
                            self.wire_armed_port = Some((
                                PortEndpoint {
                                    node_path: pnode.path.clone(),
                                    node_label: pnode.label.clone(),
                                    port_name: pp.name.clone(),
                                    port_type: pp.type_label.clone(),
                                    direction: pp.direction,
                                    is_conjugated: pp.is_conjugated,
                                    world_pos: Pos2::new(pp.anchor.x, pp.anchor.y),
                                },
                                None,
                            ));
                        }
                    }
                    ui.close();
                }
                if ui.button("Reset Port Position (Auto)").clicked() {
                    action = Some(CanvasAction::MovePort {
                        node_key: node_key.clone(),
                        port_name: port_name.clone(),
                        placement: None,
                    });
                    ui.close();
                }
                ui.separator();
                if ui.button("Toggle Conjugation (~)").clicked() {
                    action = Some(CanvasAction::ApplyCommand(
                        SyssurCommand::TogglePortConjugation {
                            file: port_file.clone(),
                            port_name: port_name.clone(),
                        },
                    ));
                    ui.close();
                }
                if ui
                    .button(
                        RichText::new("Delete Port")
                            .color(Color32::from_rgb(245, 115, 115)),
                    )
                    .clicked()
                {
                    action = Some(CanvasAction::ApplyCommand(SyssurCommand::DeleteElement {
                        file: port_file.clone(),
                        kind_hint: Some("port".to_string()),
                        name: port_name.clone(),
                    }));
                    ui.close();
                }
                return;
            }

            if let Some(edge_idx) = self.context_edge {
                ui.set_min_width(220.0);
                if let Some(edge) = layout.edges.get(edge_idx) {
                    let title = edge
                        .label
                        .as_deref()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| format!("{:?}: {l}", edge.kind))
                        .unwrap_or_else(|| format!("{:?} Connector", edge.kind));
                    ui.label(RichText::new(title).strong());
                    ui.separator();
                    if ui
                        .button(
                            RichText::new("Delete Connector")
                                .color(Color32::from_rgb(245, 115, 115)),
                        )
                        .clicked()
                    {
                        action = Some(CanvasAction::DeleteEdge { index: edge_idx });
                        ui.close();
                    }
                }
                return;
            }

            ui.set_min_width(250.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.context_search)
                    .hint_text("Search SysML v2 (create, relate, transform)..."),
            );
            let q = self.context_search.trim().to_ascii_lowercase();
            ui.separator();

            if let Some(ref ctx_path) = self.context_node.clone() {
                let key = ctx_path.canonical_key();
                if let Some(node) = layout.nodes.get(&key) {
                    let meta = meta_by_key.get(&key);
                    let header = match meta.and_then(|m| m.type_name.as_deref()) {
                        Some(tn) => format!("{} : {tn}", node.label),
                        None => node.label.clone(),
                    };
                    ui.label(RichText::new(header).strong());
                    ui.separator();

                    let file = meta
                        .map(|m| m.source_file.clone())
                        .unwrap_or_else(|| active_file.to_string());
                    let elem_name = ctx_path
                        .leaf()
                        .map(|l| l.short_name().to_string())
                        .unwrap_or_else(|| node.label.clone());

                    let ctx_world = self
                        .context_world_pos
                        .map(|w| syssur_layout::Point2 { x: w.x, y: w.y });
                    let inside_ctx = CreateContext::InsideNode(node.shape);
                    let can_create_inside = has_items_for_context(inside_ctx);
                    if !q.is_empty() {
                        if can_create_inside {
                            if let Some(item) = show_create_submenus(ui, &q, inside_ctx) {
                                action = Some(CanvasAction::PlaceCatalogItem {
                                    item,
                                    container: Some(ctx_path.clone()),
                                    file: file.clone(),
                                    world_pos: ctx_world,
                                });
                                ui.close();
                            }
                        }
                    } else {
                        if can_create_inside {
                            ui.menu_button("Create inside...", |ui| {
                                if let Some(item) = show_create_submenus(ui, "", inside_ctx) {
                                    action = Some(CanvasAction::PlaceCatalogItem {
                                        item,
                                        container: Some(ctx_path.clone()),
                                        file: file.clone(),
                                        world_pos: ctx_world,
                                    });
                                    ui.close();
                                }
                            });
                        }

                        ui.menu_button("Connect / Relate...", |ui| {
                            ui.set_min_width(220.0);
                            for rel in catalog_relationships() {
                                if catalog_item_row(ui, rel, false, false).clicked() {
                                    self.rel_pick = Some(RelationshipPickState {
                                        rel_keyword: rel.keyword,
                                        rel_label: rel.label,
                                        source_path: ctx_path.clone(),
                                        source_label: node.label.clone(),
                                    });
                                    ui.close();
                                }
                            }
                        });

                        ui.menu_button("Transform / Refactor...", |ui| {
                            ui.set_min_width(210.0);
                            if ui.button("Convert Def <-> Usage").clicked() {
                                action = Some(CanvasAction::ApplyCommand(
                                    SyssurCommand::ToggleDefUsage {
                                        file: file.clone(),
                                        element_name: elem_name.clone(),
                                    },
                                ));
                                ui.close();
                            }
                            if ui.button("Extract Definition").clicked() {
                                let pkg = ctx_path
                                    .0
                                    .first()
                                    .and_then(|id| id.as_str().split("::").next())
                                    .unwrap_or("Model")
                                    .to_string();
                                action = Some(CanvasAction::ApplyCommand(
                                    SyssurCommand::ExtractDefinition {
                                        file: file.clone(),
                                        package: pkg,
                                        usage_name: elem_name.clone(),
                                        def_name: format!("{elem_name}Def"),
                                    },
                                ));
                                ui.close();
                            }
                            ui.menu_button("Toggle Modifier", |ui| {
                                for m in [
                                    "abstract",
                                    "variation",
                                    "variant",
                                    "individual",
                                    "ref",
                                    "derived",
                                    "constant",
                                    "parallel",
                                ] {
                                    if ui.button(m).clicked() {
                                        action = Some(CanvasAction::ApplyCommand(
                                            SyssurCommand::ToggleModifier {
                                                file: file.clone(),
                                                element_name: elem_name.clone(),
                                                modifier: m.to_string(),
                                            },
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button("Visibility", |ui| {
                                for (lbl, v) in [
                                    ("default", None),
                                    ("public", Some("public")),
                                    ("private", Some("private")),
                                    ("protected", Some("protected")),
                                ] {
                                    if ui.button(lbl).clicked() {
                                        action = Some(CanvasAction::ApplyCommand(
                                            SyssurCommand::SetVisibility {
                                                file: file.clone(),
                                                element_name: elem_name.clone(),
                                                visibility: v.map(str::to_string),
                                            },
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button("Direction", |ui| {
                                for (lbl, d) in [
                                    ("none", None),
                                    ("in", Some("in")),
                                    ("out", Some("out")),
                                    ("inout", Some("inout")),
                                ] {
                                    if ui.button(lbl).clicked() {
                                        action = Some(CanvasAction::ApplyCommand(
                                            SyssurCommand::SetDirection {
                                                file: file.clone(),
                                                element_name: elem_name.clone(),
                                                direction: d.map(str::to_string),
                                            },
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            ui.menu_button("Change Kind...", |ui| {
                                for kw in [
                                    "part",
                                    "item",
                                    "occurrence",
                                    "attribute",
                                    "action",
                                    "state",
                                    "calc",
                                    "constraint",
                                    "requirement",
                                    "concern",
                                    "case",
                                    "use case",
                                    "analysis",
                                    "verification",
                                ] {
                                    if ui.button(kw).clicked() {
                                        action = Some(CanvasAction::ApplyCommand(
                                            SyssurCommand::ChangeElementKind {
                                                file: file.clone(),
                                                element_name: elem_name.clone(),
                                                new_keyword: kw.to_string(),
                                            },
                                        ));
                                        ui.close();
                                    }
                                }
                            });
                            if !package_names.is_empty() {
                                ui.menu_button("Move to Package...", |ui| {
                                    for pkg in package_names {
                                        if ui.button(pkg).clicked() {
                                            action = Some(CanvasAction::ApplyCommand(
                                                SyssurCommand::MoveElementToPackage {
                                                    file: file.clone(),
                                                    element_name: elem_name.clone(),
                                                    target_package: pkg.clone(),
                                                },
                                            ));
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                    }

                    ui.separator();
                    let target_def = meta
                        .and_then(|m| m.type_def.clone())
                        .or_else(|| ctx_path.leaf().cloned());
                    if let Some(def_id) = target_def {
                        if q.is_empty() && ui.button(tr(locale, TextKey::CtxInspectAsRoot)).clicked()
                        {
                            action = Some(CanvasAction::InspectAsRoot { target: def_id });
                            ui.close();
                        }
                    }
                    if (q.is_empty() || "resize".contains(&q) || "size".contains(&q))
                        && ui.button("Reset Block Size (Auto)").clicked()
                    {
                        action = Some(CanvasAction::ResetNodeSize {
                            node_key: key.clone(),
                        });
                        ui.close();
                    }
                    if let Some(leaf) = ctx_path.leaf().cloned() {
                        if (q.is_empty() || "rename".contains(&q))
                            && ui.button(tr(locale, TextKey::CtxRenameElement)).clicked()
                        {
                            action = Some(CanvasAction::RequestRename {
                                element: leaf.clone(),
                                file: file.clone(),
                            });
                            ui.close();
                        }
                        if (q.is_empty() || "delete".contains(&q) || "remove".contains(&q))
                            && ui
                                .button(
                                    RichText::new(tr(locale, TextKey::DeleteElement))
                                        .color(Color32::from_rgb(245, 115, 115)),
                                )
                                .clicked()
                        {
                            action = Some(CanvasAction::DeleteElement {
                                element: leaf,
                                file: file.clone(),
                            });
                            ui.close();
                        }
                    }
                    if q.is_empty() {
                        if ui.button(tr(locale, TextKey::CtxJumpToSource)).clicked() {
                            action = Some(CanvasAction::JumpToSource { file });
                            ui.close();
                        }
                        if ui.button(tr(locale, TextKey::CtxCopyPath)).clicked() {
                            action = Some(CanvasAction::CopyPath {
                                path: ctx_path.to_string(),
                            });
                            ui.close();
                        }
                    }
                }
            } else {
                // Empty canvas context menu
                let ctx_world = self
                    .context_world_pos
                    .map(|w| syssur_layout::Point2 { x: w.x, y: w.y });
                if !q.is_empty() {
                    if let Some(item) = show_create_submenus(ui, &q, CreateContext::EmptyCanvas) {
                        action = Some(CanvasAction::PlaceCatalogItem {
                            item,
                            container: None,
                            file: active_file.to_string(),
                            world_pos: ctx_world,
                        });
                        ui.close();
                    }
                } else {
                    ui.menu_button("Create SysML v2 Element...", |ui| {
                        if let Some(item) =
                            show_create_submenus(ui, "", CreateContext::EmptyCanvas)
                        {
                            action = Some(CanvasAction::PlaceCatalogItem {
                                item,
                                container: None,
                                file: active_file.to_string(),
                                world_pos: ctx_world,
                            });
                            ui.close();
                        }
                    });
                    ui.separator();
                    if ui.button(tr(locale, TextKey::AutoLayout)).clicked() {
                        action = Some(CanvasAction::AutoLayout);
                        ui.close();
                    }
                    if ui.button(tr(locale, TextKey::CtxFitView)).clicked() {
                        action = Some(CanvasAction::FitView);
                        ui.close();
                    }
                }
            }
        });

        // HUD overlay
        let tool_hint = if let Some((ref src_ep, rel_kw)) = self.wire_armed_port {
            let kind_str = rel_kw.unwrap_or("connect");
            format!(
                " | Wiring ({kind_str}) from {}.{}: click target port (Esc to cancel)",
                src_ep.node_label, src_ep.port_name
            )
        } else if let Some(ref pick) = self.rel_pick {
            format!(" | Wiring {}: click target node (Esc to cancel)", pick.rel_label)
        } else if let Some(t) = active_tool {
            format!(" | Tool: {} (click canvas/container, Esc to cancel)", t.label)
        } else {
            " | Drag node or port to move | Shift+Drag port to wire | Drag bg to pan".to_string()
        };
        clipped.text(
            avail.left_bottom() + Vec2::new(10.0, -10.0),
            Align2::LEFT_BOTTOM,
            format!(
                "LOD: {:?} | Zoom: {:.0}% | Visible: {}/{} nodes, {}/{} edges{}",
                lod,
                self.zoom * 100.0,
                self.last_visible_nodes,
                layout.nodes.len(),
                self.last_visible_edges,
                layout.edges.len(),
                tool_hint,
            ),
            FontId::monospace(11.5),
            Color32::from_gray(155),
        );

        action
    }

    fn selection_handle_positions(&self, node: &PositionedNode, origin: Pos2) -> [Pos2; 4] {
        let rect = Rect::from_min_size(
            self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
            Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
        );
        let pad = 11.0;
        [
            Pos2::new(rect.center().x, rect.top() - pad),
            Pos2::new(rect.right() + pad, rect.center().y),
            Pos2::new(rect.center().x, rect.bottom() + pad),
            Pos2::new(rect.left() - pad, rect.center().y),
        ]
    }

    fn is_resizable_shape(shape: NodeShape) -> bool {
        matches!(
            shape,
            NodeShape::PartBlock
                | NodeShape::ActionBox
                | NodeShape::StateBox
                | NodeShape::RequirementCard
                | NodeShape::ConstraintCard
                | NodeShape::CaseOval
                | NodeShape::GenericElement
        )
    }

    fn resize_handle_positions(
        &self,
        node: &PositionedNode,
        origin: Pos2,
    ) -> [(ResizeCorner, Pos2); 4] {
        let rect = Rect::from_min_size(
            self.world_to_screen(Pos2::new(node.rect.x, node.rect.y), origin),
            Vec2::new(node.rect.w * self.zoom, node.rect.h * self.zoom),
        );
        [
            (ResizeCorner::TopLeft, rect.left_top()),
            (ResizeCorner::TopRight, rect.right_top()),
            (ResizeCorner::BottomRight, rect.right_bottom()),
            (ResizeCorner::BottomLeft, rect.left_bottom()),
        ]
    }

    fn hit_test_resize_handle<'a>(
        &self,
        layout: &'a LayoutOutput,
        screen_pos: Pos2,
        origin: Pos2,
    ) -> Option<(&'a PositionedNode, ResizeCorner)> {
        let sel = self.selected.as_ref()?;
        let node = layout.nodes.get(&sel.canonical_key())?;
        if !Self::is_resizable_shape(node.shape) {
            return None;
        }
        self.resize_handle_positions(node, origin)
            .into_iter()
            .find(|(_, pos)| pos.distance(screen_pos) <= 8.0)
            .map(|(corner, _)| (node, corner))
    }

    fn paint_selection_handles(&self, painter: &egui::Painter, node: &PositionedNode, origin: Pos2) {
        if Self::is_resizable_shape(node.shape) {
            for (_, corner_pos) in self.resize_handle_positions(node, origin) {
                let hrect = Rect::from_center_size(corner_pos, Vec2::splat(7.0));
                painter.rect_filled(hrect, 1.5, Color32::from_rgb(235, 245, 255));
                painter.rect_stroke(
                    hrect,
                    1.5,
                    Stroke::new(1.3, Color32::from_rgb(70, 155, 235)),
                    StrokeKind::Outside,
                );
            }
        }
        let handles = self.selection_handle_positions(node, origin);
        let dirs = [
            Vec2::new(0.0, -1.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(-1.0, 0.0),
        ];
        for (pos, dir) in handles.iter().zip(dirs.iter()) {
            painter.circle_filled(*pos, 6.0, Color32::from_rgb(45, 56, 72));
            painter.circle_stroke(*pos, 6.0, Stroke::new(1.2, Color32::from_rgb(110, 200, 245)));
            let tip = *pos + *dir * 3.5;
            let perp = Vec2::new(-dir.y, dir.x);
            let left = *pos - *dir * 2.0 + perp * 2.5;
            let right = *pos - *dir * 2.0 - perp * 2.5;
            painter.line_segment([left, tip], Stroke::new(1.3, Color32::from_rgb(200, 235, 255)));
            painter.line_segment([right, tip], Stroke::new(1.3, Color32::from_rgb(200, 235, 255)));
        }
    }

    fn hit_test_selection_handle<'a>(
        &self,
        layout: &'a LayoutOutput,
        screen_pos: Pos2,
        origin: Pos2,
    ) -> Option<&'a PositionedNode> {
        let sel = self.selected.as_ref()?;
        let node = layout.nodes.get(&sel.canonical_key())?;
        let handles = self.selection_handle_positions(node, origin);
        if handles.iter().any(|h| h.distance(screen_pos) <= 10.0) {
            Some(node)
        } else {
            None
        }
    }

    fn hit_test_node_static<'a>(
        pan: Vec2,
        zoom: f32,
        nodes: &[&'a PositionedNode],
        screen_pos: Pos2,
        origin: Pos2,
    ) -> Option<&'a PositionedNode> {
        for node in nodes.iter().rev() {
            let screen_rect = Rect::from_min_size(
                origin + pan + Vec2::new(node.rect.x, node.rect.y) * zoom,
                Vec2::new(node.rect.w * zoom, node.rect.h * zoom),
            );
            if screen_rect.contains(screen_pos) {
                return Some(node);
            }
        }
        None
    }

    fn hit_test_edge(&self, edge: &RoutedEdge, screen_pos: Pos2, origin: Pos2) -> bool {
        let pts: Vec<Pos2> = edge
            .waypoints
            .iter()
            .map(|w| self.world_to_screen(Pos2::new(w.x, w.y), origin))
            .collect();
        for seg in pts.windows(2) {
            if point_segment_distance(screen_pos, seg[0], seg[1]) <= 9.0 {
                return true;
            }
        }
        false
    }

    fn hit_test_port(
        &self,
        nodes: &[&PositionedNode],
        screen_pos: Pos2,
        origin: Pos2,
    ) -> Option<PortEndpoint> {
        Self::hit_test_port_static(self.pan, self.zoom, nodes, screen_pos, origin)
    }

    fn hit_test_port_static(
        pan: Vec2,
        zoom: f32,
        nodes: &[&PositionedNode],
        screen_pos: Pos2,
        origin: Pos2,
    ) -> Option<PortEndpoint> {
        let hit_radius = (14.0 * zoom).clamp(10.0, 22.0);
        for node in nodes.iter().rev() {
            for port in &node.ports {
                let world_pos = Pos2::new(port.anchor.x, port.anchor.y);
                let screen_anchor = origin + pan + world_pos.to_vec2() * zoom;
                if screen_anchor.distance(screen_pos) <= hit_radius {
                    return Some(PortEndpoint {
                        node_path: node.path.clone(),
                        node_label: node.label.clone(),
                        port_name: port.name.clone(),
                        port_type: port.type_label.clone(),
                        direction: port.direction,
                        is_conjugated: port.is_conjugated,
                        world_pos,
                    });
                }
            }
        }
        None
    }

    fn paint_wire_draft(&self, painter: &egui::Painter, draft: &WireDraft, origin: Pos2) {
        let start = self.world_to_screen(draft.source.world_pos, origin);
        let rel_prefix = draft
            .rel_keyword
            .map(|kw| format!("[{kw}] "))
            .unwrap_or_default();
        let (end_world, color, hint) = match &draft.hovered_target {
            Some((target, PortCompatibility::Compatible)) => {
                let st = draft.source.port_type.as_deref().unwrap_or("Port");
                let tt = target.port_type.as_deref().unwrap_or("Port");
                (
                    target.world_pos,
                    Color32::from_rgb(85, 220, 130),
                    format!("{rel_prefix}Compatible: {st} -> {tt}"),
                )
            }
            Some((target, PortCompatibility::DirectionMismatch { source_dir, target_dir })) => (
                target.world_pos,
                Color32::from_rgb(245, 180, 70),
                format!("Direction mismatch: {source_dir:?} -> {target_dir:?} (use ~)"),
            ),
            Some((target, PortCompatibility::TypeMismatch { source_type, target_type })) => (
                target.world_pos,
                Color32::from_rgb(245, 90, 90),
                format!("Type mismatch: {source_type} != {target_type}"),
            ),
            None => (
                draft.current_world,
                Color32::from_rgb(130, 195, 255),
                format!(
                    "Wiring from {}.{}...",
                    draft.source.node_label, draft.source.port_name
                ),
            ),
        };
        let end = self.world_to_screen(end_world, origin);
        let mid_x = (start.x + end.x) * 0.5;
        let pts = [start, Pos2::new(mid_x, start.y), Pos2::new(mid_x, end.y), end];
        for seg in pts.windows(2) {
            painter.line_segment([seg[0], seg[1]], Stroke::new(2.2, color));
        }
        if draft.hovered_target.is_some() {
            painter.circle_stroke(end, 10.0, Stroke::new(2.2, color));
        }
        painter.text(
            end + Vec2::new(12.0, -12.0),
            Align2::LEFT_BOTTOM,
            hint,
            FontId::monospace(11.0),
            color,
        );
    }

    fn paint_handle_drag(&self, painter: &egui::Painter, drag: &NodeHandleDrag, origin: Pos2) {
        let start = self.world_to_screen(drag.start_world, origin);
        let (end_world, color, hint) = match &drag.hovered_target {
            Some((_, lbl, _, pos)) => (
                *pos,
                Color32::from_rgb(90, 220, 150),
                format!("Link {} -> {}", drag.source_label, lbl),
            ),
            None => (
                drag.current_world,
                Color32::from_rgb(135, 195, 255),
                format!("Connecting from {}...", drag.source_label),
            ),
        };
        let end = self.world_to_screen(end_world, origin);
        paint_dashed_line(painter, start, end, Stroke::new(2.0, color));
        paint_arrowhead(painter, start, end, color);
        painter.text(
            end + Vec2::new(10.0, -10.0),
            Align2::LEFT_BOTTOM,
            hint,
            FontId::monospace(11.0),
            color,
        );
    }

    fn world_to_screen(&self, p: Pos2, origin: Pos2) -> Pos2 {
        origin + self.pan + p.to_vec2() * self.zoom
    }

    fn screen_to_world(&self, p: Pos2, origin: Pos2) -> Pos2 {
        (((p - origin) - self.pan) / self.zoom).to_pos2()
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_node(
        &self,
        painter: &egui::Painter,
        node: &PositionedNode,
        meta: Option<&NodeMeta>,
        rect: Rect,
        origin: Pos2,
        lod: RenderLod,
        selected: bool,
        is_container: bool,
        direction: LayoutDirection,
        paint_ports: bool,
    ) {
        let sel_stroke = Color32::from_rgb(96, 175, 255);
        match node.shape {
            NodeShape::StartCircle => {
                let r = (rect.width().min(rect.height()) * 0.42).max(6.0);
                let c = rect.center();
                painter.circle_filled(c, r, Color32::from_rgb(230, 238, 248));
                if selected {
                    painter.circle_stroke(c, r + 3.0, Stroke::new(2.0, sel_stroke));
                }
                return;
            }
            NodeShape::DoneBullseye => {
                let r = (rect.width().min(rect.height()) * 0.44).max(7.0);
                let c = rect.center();
                painter.circle_stroke(
                    c,
                    r,
                    Stroke::new(2.0, if selected { sel_stroke } else { Color32::from_rgb(230, 238, 248) }),
                );
                painter.circle_filled(c, r * 0.56, Color32::from_rgb(230, 238, 248));
                return;
            }
            NodeShape::ForkJoinBar => {
                let bar = if direction == LayoutDirection::TopToBottom {
                    Rect::from_center_size(
                        rect.center(),
                        Vec2::new(rect.width(), (7.0 * self.zoom).clamp(4.0, 10.0)),
                    )
                } else {
                    Rect::from_center_size(
                        rect.center(),
                        Vec2::new((7.0 * self.zoom).clamp(4.0, 10.0), rect.height()),
                    )
                };
                painter.rect_filled(bar, 2.0, Color32::from_rgb(225, 234, 246));
                if selected {
                    painter.rect_stroke(bar.expand(2.5), 2.0, Stroke::new(1.8, sel_stroke), StrokeKind::Outside);
                }
                if lod == RenderLod::Micro {
                    painter.text(
                        bar.right_center() + Vec2::new(6.0, 0.0),
                        Align2::LEFT_CENTER,
                        &node.label,
                        FontId::monospace((10.5 * self.zoom).clamp(8.5, 13.0)),
                        Color32::from_gray(175),
                    );
                }
                return;
            }
            NodeShape::DecideDiamond | NodeShape::MergeDiamond => {
                let c = rect.center();
                let rx = rect.width() * 0.48;
                let ry = rect.height() * 0.48;
                let pts = vec![
                    Pos2::new(c.x, c.y - ry),
                    Pos2::new(c.x + rx, c.y),
                    Pos2::new(c.x, c.y + ry),
                    Pos2::new(c.x - rx, c.y),
                ];
                let fill = if node.shape == NodeShape::MergeDiamond {
                    Color32::from_rgb(195, 215, 240)
                } else {
                    Color32::from_rgb(28, 34, 44)
                };
                let stroke = if selected {
                    Stroke::new(2.2, sel_stroke)
                } else {
                    Stroke::new(1.6, Color32::from_rgb(195, 215, 240))
                };
                painter.add(egui::Shape::convex_polygon(pts, fill, stroke));
                if lod == RenderLod::Micro {
                    painter.text(
                        Pos2::new(c.x - rx - 6.0, c.y),
                        Align2::RIGHT_CENTER,
                        &node.label,
                        FontId::monospace((10.5 * self.zoom).clamp(8.5, 13.0)),
                        Color32::from_gray(180),
                    );
                }
                return;
            }
            NodeShape::ActionBox => {
                let radius = CornerRadius::same((14.0 * self.zoom).clamp(4.0, 18.0) as u8);
                let fill = if is_container {
                    Color32::from_rgba_unmultiplied(28, 38, 48, 220)
                } else {
                    Color32::from_rgb(34, 52, 62)
                };
                let border = if selected {
                    sel_stroke
                } else {
                    Color32::from_rgb(92, 182, 178)
                };
                painter.rect_filled(rect, radius, fill);
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(if selected { 2.2 } else { 1.4 }, border),
                    StrokeKind::Inside,
                );
                if lod != RenderLod::Macro {
                    painter.text(
                        rect.left_top() + Vec2::new(10.0 * self.zoom, 7.0 * self.zoom),
                        Align2::LEFT_TOP,
                        format!("«action» {}", node.label),
                        FontId::proportional((13.0 * self.zoom).clamp(9.5, 18.0)),
                        Color32::from_rgb(225, 244, 242),
                    );
                }
                return;
            }
            NodeShape::StateBox => {
                let is_def = meta.is_some_and(|m| m.is_def);
                let radius = if is_def {
                    CornerRadius::ZERO
                } else {
                    CornerRadius::same((14.0 * self.zoom).clamp(5.0, 18.0) as u8)
                };
                let fill = if is_container {
                    Color32::from_rgba_unmultiplied(38, 30, 56, 220)
                } else {
                    Color32::from_rgb(48, 38, 70)
                };
                let border = if selected {
                    sel_stroke
                } else {
                    Color32::from_rgb(175, 135, 245)
                };
                painter.rect_filled(rect, radius, fill);
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(if selected { 2.2 } else { 1.5 }, border),
                    StrokeKind::Inside,
                );
                if lod != RenderLod::Macro {
                    let stereo = meta
                        .map(|m| m.stereotype.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(if is_def { "«state def»" } else { "«state»" });
                    painter.text(
                        Pos2::new(rect.center().x, rect.top() + 5.0 * self.zoom),
                        Align2::CENTER_TOP,
                        stereo,
                        FontId::proportional((10.5 * self.zoom).clamp(8.5, 13.0)),
                        Color32::from_rgb(200, 175, 250),
                    );
                    painter.text(
                        Pos2::new(rect.center().x, rect.top() + 19.0 * self.zoom),
                        Align2::CENTER_TOP,
                        &node.label,
                        FontId::proportional((13.0 * self.zoom).clamp(9.5, 18.0)),
                        Color32::from_rgb(235, 225, 255),
                    );
                }
                return;
            }
            NodeShape::RequirementCard | NodeShape::ConstraintCard => {
                let is_def = meta.is_some_and(|m| m.is_def);
                let radius = if is_def {
                    CornerRadius::ZERO
                } else {
                    CornerRadius::same((10.0 * self.zoom).clamp(4.0, 13.0) as u8)
                };
                let (fill, accent, default_stereo) = if node.shape == NodeShape::ConstraintCard {
                    (
                        Color32::from_rgb(36, 44, 40),
                        Color32::from_rgb(135, 215, 175),
                        if is_def {
                            "«constraint def»"
                        } else {
                            "«constraint»"
                        },
                    )
                } else {
                    (
                        Color32::from_rgb(46, 40, 30),
                        Color32::from_rgb(235, 185, 80),
                        if is_def {
                            "«requirement def»"
                        } else {
                            "«requirement»"
                        },
                    )
                };
                let stereo = meta
                    .map(|m| m.stereotype.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(default_stereo);
                painter.rect_filled(rect, radius, fill);
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(
                        if selected { 2.2 } else { 1.4 },
                        if selected { sel_stroke } else { accent },
                    ),
                    StrokeKind::Inside,
                );
                if lod == RenderLod::Macro {
                    return;
                }
                let id_badge = meta
                    .and_then(|m| m.short_id.as_deref())
                    .map(|id| format!("<{id}> "))
                    .unwrap_or_default();
                painter.text(
                    Pos2::new(rect.center().x, rect.top() + 5.0 * self.zoom),
                    Align2::CENTER_TOP,
                    stereo,
                    FontId::proportional((10.5 * self.zoom).clamp(8.5, 13.0)),
                    accent,
                );
                painter.text(
                    Pos2::new(rect.center().x, rect.top() + 19.0 * self.zoom),
                    Align2::CENTER_TOP,
                    format!("{id_badge}{}", node.label),
                    FontId::proportional((13.5 * self.zoom).clamp(9.5, 18.0)),
                    Color32::from_gray(242),
                );
                if lod == RenderLod::Micro {
                    if let Some(doc) = meta.and_then(|m| m.doc.as_deref()) {
                        let div_y = rect.top() + 37.0 * self.zoom;
                        painter.line_segment(
                            [Pos2::new(rect.left(), div_y), Pos2::new(rect.right(), div_y)],
                            Stroke::new(1.0, Color32::from_rgba_unmultiplied(180, 180, 180, 65)),
                        );
                        let snippet: String = doc.chars().take(46).collect();
                        let suffix = if doc.chars().count() > 46 { "..." } else { "" };
                        painter.text(
                            rect.left_top() + Vec2::new(8.0 * self.zoom, 41.0 * self.zoom),
                            Align2::LEFT_TOP,
                            format!("{snippet}{suffix}"),
                            FontId::proportional((11.0 * self.zoom).clamp(8.5, 14.0)),
                            Color32::from_gray(195),
                        );
                    }
                }
                return;
            }
            NodeShape::CaseOval => {
                let radius = CornerRadius::same((rect.height() * 0.45).clamp(10.0, 28.0) as u8);
                painter.rect_filled(rect, radius, Color32::from_rgb(38, 44, 62));
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(
                        if selected { 2.2 } else { 1.4 },
                        if selected {
                            sel_stroke
                        } else {
                            Color32::from_rgb(145, 185, 245)
                        },
                    ),
                    StrokeKind::Inside,
                );
                if lod != RenderLod::Macro {
                    let stereo = meta
                        .map(|m| m.stereotype.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("«case»");
                    painter.text(
                        rect.center_top() + Vec2::new(0.0, 6.0 * self.zoom),
                        Align2::CENTER_TOP,
                        stereo,
                        FontId::monospace((10.0 * self.zoom).clamp(8.0, 12.5)),
                        Color32::from_rgb(145, 185, 245),
                    );
                    painter.text(
                        rect.center() + Vec2::new(0.0, 6.0 * self.zoom),
                        Align2::CENTER_CENTER,
                        &node.label,
                        FontId::proportional((13.0 * self.zoom).clamp(9.5, 17.0)),
                        Color32::from_gray(240),
                    );
                }
                return;
            }
            NodeShape::GenericElement => {
                let is_def = meta.is_some_and(|m| m.is_def);
                let radius = if is_def {
                    CornerRadius::ZERO
                } else {
                    CornerRadius::same((10.0 * self.zoom).clamp(4.0, 13.0) as u8)
                };
                painter.rect_filled(rect, radius, Color32::from_rgb(35, 39, 48));
                painter.rect_stroke(
                    rect,
                    radius,
                    Stroke::new(
                        if selected { 2.2 } else { 1.2 },
                        if selected {
                            sel_stroke
                        } else {
                            Color32::from_rgb(125, 140, 165)
                        },
                    ),
                    StrokeKind::Inside,
                );
                if lod != RenderLod::Macro {
                    let stereo = meta
                        .map(|m| m.stereotype.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("«element»");
                    painter.text(
                        Pos2::new(rect.center().x, rect.top() + 5.0 * self.zoom),
                        Align2::CENTER_TOP,
                        stereo,
                        FontId::proportional((10.5 * self.zoom).clamp(8.5, 12.5)),
                        Color32::from_rgb(160, 185, 220),
                    );
                    let title = match meta.and_then(|m| m.type_name.as_deref()) {
                        Some(tn) => format!("{} : {tn}", node.label),
                        None => node.label.clone(),
                    };
                    painter.text(
                        Pos2::new(rect.center().x, rect.top() + 19.0 * self.zoom),
                        Align2::CENTER_TOP,
                        title,
                        FontId::proportional((13.0 * self.zoom).clamp(9.5, 17.0)),
                        Color32::from_gray(236),
                    );
                }
                return;
            }
            NodeShape::PartBlock => {}
        }

        // SysML v2 Table 4 PartBlock rendering:
        // Sharp square corners for definitions (`part def`), rounded corners for usages (`part`).
        let is_def = meta.is_some_and(|m| m.is_def);
        let origin_kind = meta.map(|m| &m.origin);
        let is_inherited = matches!(origin_kind, Some(FeatureOrigin::Inherited(_)));

        let fill = if is_container {
            Color32::from_rgba_unmultiplied(30, 36, 46, 215)
        } else if is_inherited {
            Color32::from_rgb(30, 34, 42)
        } else {
            Color32::from_rgb(36, 43, 54)
        };
        let border_color = if selected {
            sel_stroke
        } else if matches!(origin_kind, Some(FeatureOrigin::Redefined(_))) {
            Color32::from_rgb(235, 170, 75)
        } else if matches!(origin_kind, Some(FeatureOrigin::Subsetted(_))) {
            Color32::from_rgb(120, 200, 165)
        } else {
            Color32::from_rgb(86, 102, 124)
        };

        let radius = if is_def {
            CornerRadius::ZERO
        } else {
            CornerRadius::same((11.0 * self.zoom).clamp(4.0, 14.0) as u8)
        };
        painter.rect_filled(rect, radius, fill);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(if selected { 2.2 } else { 1.3 }, border_color),
            StrokeKind::Inside,
        );

        if lod == RenderLod::Macro {
            return;
        }

        // Line 1: Centered SysML v2 stereotype guillemets («part def», «abstract part def», «part», ...)
        let stereo = meta
            .map(|m| m.stereotype.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(if is_def { "«part def»" } else { "«part»" });
        let stereo_color = if is_def {
            Color32::from_rgb(155, 195, 235)
        } else {
            Color32::from_rgb(175, 205, 195)
        };
        painter.text(
            Pos2::new(rect.center().x, rect.top() + 5.0 * self.zoom),
            Align2::CENTER_TOP,
            stereo,
            FontId::proportional((10.5 * self.zoom).clamp(8.5, 13.0)),
            stereo_color,
        );

        // Line 2: Centered Name Compartment header (<shortId> ^name : Type [mult] or Name :> Super)
        let short_badge = meta
            .and_then(|m| m.short_id.as_deref())
            .map(|s| format!("<{s}> "))
            .unwrap_or_default();
        let inh_prefix = if is_inherited { "^" } else { "" };
        let type_or_spec = if is_def {
            meta.and_then(|m| m.specializes.as_deref())
                .map(|sp| format!(" :> {sp}"))
                .unwrap_or_default()
        } else {
            meta.and_then(|m| m.type_name.as_deref())
                .map(|tn| format!(" : {tn}"))
                .unwrap_or_default()
        };
        let mult_suffix = meta
            .and_then(|m| m.multiplicity)
            .map(|m| format!(" [{m}]"))
            .unwrap_or_default();
        let header_line = format!(
            "{short_badge}{inh_prefix}{}{type_or_spec}{mult_suffix}",
            node.label
        );
        let header_color = if is_inherited {
            Color32::from_gray(175)
        } else {
            Color32::from_gray(242)
        };
        painter.text(
            Pos2::new(rect.center().x, rect.top() + 19.0 * self.zoom),
            Align2::CENTER_TOP,
            header_line,
            FontId::proportional((13.5 * self.zoom).clamp(10.0, 19.0)),
            header_color,
        );

        // Line 3 (optional): Subsets / Redefines subtitle
        let origin_sub = match origin_kind {
            Some(FeatureOrigin::Subsetted(t)) => Some(format!("subsets {}", t.short_name())),
            Some(FeatureOrigin::Redefined(t)) => Some(format!("redefines {}", t.short_name())),
            _ => None,
        };
        if let Some(ref sub) = origin_sub {
            painter.text(
                Pos2::new(rect.center().x, rect.top() + 36.0 * self.zoom),
                Align2::CENTER_TOP,
                sub,
                FontId::proportional((10.5 * self.zoom).clamp(8.5, 13.0)),
                Color32::from_rgb(175, 200, 185),
            );
        }

        let name_comp_h = if origin_sub.is_some() { 52.0 } else { 38.0 };
        let attrs = meta.map(|m| m.attributes.as_slice()).unwrap_or(&[]);

        if lod == RenderLod::Micro {
            // Horizontal compartment divider separating Name Compartment from compartments below
            if is_container || !attrs.is_empty() {
                let div_y = (rect.top() + name_comp_h * self.zoom).min(rect.bottom() - 4.0);
                painter.line_segment(
                    [Pos2::new(rect.left(), div_y), Pos2::new(rect.right(), div_y)],
                    Stroke::new(1.0, border_color),
                );
            }

            if !attrs.is_empty() {
                let div_y = rect.top() + name_comp_h * self.zoom;
                painter.text(
                    Pos2::new(rect.center().x, div_y + 3.0 * self.zoom),
                    Align2::CENTER_TOP,
                    "attributes",
                    FontId::proportional((10.5 * self.zoom).clamp(8.5, 13.0)),
                    Color32::from_gray(165),
                );
                let mut y_off = div_y + 18.0 * self.zoom;
                for attr in attrs.iter().take(8) {
                    if y_off + 12.0 * self.zoom > rect.bottom() {
                        break;
                    }
                    let attr_col = if attr.starts_with('^') {
                        Color32::from_gray(155)
                    } else {
                        Color32::from_gray(205)
                    };
                    painter.text(
                        Pos2::new(rect.left() + 8.0 * self.zoom, y_off),
                        Align2::LEFT_TOP,
                        attr,
                        FontId::monospace((11.5 * self.zoom).clamp(8.5, 15.0)),
                        attr_col,
                    );
                    y_off += 15.0 * self.zoom;
                }
            }
        }

        if lod == RenderLod::Micro && paint_ports {
            let nkey = node.path.canonical_key();
            for port in &node.ports {
                self.paint_port(painter, &nkey, port, origin);
            }
        }
    }

    fn paint_port(
        &self,
        painter: &egui::Painter,
        node_key: &str,
        port: &PositionedPort,
        origin: Pos2,
    ) {
        let p = self.world_to_screen(Pos2::new(port.anchor.x, port.anchor.y), origin);
        let sz = 6.0 * self.zoom.clamp(0.6, 1.5);
        let box_rect = Rect::from_center_size(p, Vec2::splat(sz * 2.0));
        let port_color = if port.is_conjugated {
            Color32::from_rgb(245, 165, 95)
        } else {
            Color32::from_rgb(110, 200, 245)
        };
        let is_active = self
            .port_drag
            .as_ref()
            .is_some_and(|d| d.node_key == node_key && d.port_name == port.name)
            || self
                .selected_port
                .as_ref()
                .is_some_and(|(nk, pn)| nk == node_key && pn == &port.name)
            || self
                .wire_armed_port
                .as_ref()
                .is_some_and(|(ep, _)| ep.node_path.canonical_key() == node_key && ep.port_name == port.name);
        painter.rect_filled(box_rect, 2.0, Color32::from_rgb(25, 30, 38));
        painter.rect_stroke(
            box_rect,
            2.0,
            Stroke::new(if is_active { 2.2 } else { 1.2 }, port_color),
            StrokeKind::Inside,
        );
        if is_active {
            painter.circle_stroke(p, sz * 1.8, Stroke::new(1.4, Color32::from_rgb(255, 235, 130)));
        }

        let eff = port.direction.effective_direction(port.is_conjugated);
        let arrow = match eff {
            PortDirection::In => "->",
            PortDirection::Out => "=>",
            PortDirection::InOut => "<>",
        };
        let label = if port.is_conjugated {
            format!("~{} {arrow}", port.name)
        } else {
            format!("{} {arrow}", port.name)
        };
        let (label_offset, align) = match port.side {
            PortSide::Top => (Vec2::new(0.0, -(sz + 3.0)), Align2::CENTER_BOTTOM),
            PortSide::Bottom => (Vec2::new(0.0, sz + 3.0), Align2::CENTER_TOP),
            PortSide::Left => (Vec2::new(-(sz + 4.0), -6.0), Align2::RIGHT_CENTER),
            PortSide::Right => (Vec2::new(sz + 4.0, -6.0), Align2::LEFT_CENTER),
        };
        painter.text(
            p + label_offset,
            align,
            label,
            FontId::monospace((11.5 * self.zoom).clamp(9.0, 15.0)),
            port_color,
        );
    }

    fn edge_style(kind: RelationshipKind, selected: bool) -> (Color32, f32, bool, ArrowStyle) {
        let base_color = match kind {
            RelationshipKind::Interface => Color32::from_rgb(100, 210, 175),
            RelationshipKind::Connection => Color32::from_rgb(135, 175, 240),
            RelationshipKind::Flow => Color32::from_rgb(80, 220, 240),
            RelationshipKind::Binding => Color32::from_rgb(120, 205, 165),
            RelationshipKind::Succession => Color32::from_rgb(175, 225, 245),
            RelationshipKind::Transition => Color32::from_rgb(195, 150, 250),
            RelationshipKind::Satisfy => Color32::from_rgb(235, 185, 80),
            RelationshipKind::Verify => Color32::from_rgb(115, 225, 145),
            RelationshipKind::Allocate => Color32::from_rgb(185, 145, 245),
            RelationshipKind::Dependency => Color32::from_rgb(165, 175, 190),
            RelationshipKind::Specialization => Color32::from_rgb(220, 185, 110),
            _ => Color32::from_gray(160),
        };
        let color = if selected {
            Color32::from_rgb(255, 235, 130)
        } else {
            base_color
        };
        let stroke_w = if selected { 2.6 } else { 1.6 };
        // OMG SysML v2.0 Tables 4 & 11-15:
        // - Connection / Interface / Binding: solid line, no arrowhead (ends match by dot/name)
        // - Flow: solid line with filled triangle arrowhead
        // - Allocate: solid line with open V arrowhead + «allocate»
        // - Transition: solid line with open V arrowhead
        // - Succession: dashed line with open V arrowhead
        // - Dependency / Satisfy / Verify: dashed line with open V arrowhead
        // - Specialization: solid line with hollow triangle arrowhead
        // - Composition: solid line with filled diamond at owner end
        let is_dashed = matches!(
            kind,
            RelationshipKind::Succession
                | RelationshipKind::Satisfy
                | RelationshipKind::Verify
                | RelationshipKind::Dependency
        );
        let arrow = match kind {
            RelationshipKind::Connection | RelationshipKind::Interface | RelationshipKind::Binding => {
                ArrowStyle::None
            }
            RelationshipKind::Flow => ArrowStyle::FilledTriangle,
            RelationshipKind::Specialization => ArrowStyle::HollowTriangle,
            RelationshipKind::Composition => ArrowStyle::FilledDiamondStart,
            RelationshipKind::Allocate
            | RelationshipKind::Transition
            | RelationshipKind::Succession
            | RelationshipKind::Dependency
            | RelationshipKind::Satisfy
            | RelationshipKind::Verify
            | RelationshipKind::Subsetting
            | RelationshipKind::Redefinition
            | RelationshipKind::ReferenceSubsetting => ArrowStyle::OpenV,
        };
        (color, stroke_w, is_dashed, arrow)
    }

    fn paint_edge_geometry(
        &self,
        painter: &egui::Painter,
        edge: &RoutedEdge,
        origin: Pos2,
        selected: bool,
    ) {
        if edge.waypoints.len() < 2 {
            return;
        }
        let (color, stroke_w, is_dashed, arrow_style) = Self::edge_style(edge.kind, selected);
        let pts: Vec<Pos2> = edge
            .waypoints
            .iter()
            .map(|w| self.world_to_screen(Pos2::new(w.x, w.y), origin))
            .collect();

        for seg in pts.windows(2) {
            if is_dashed {
                paint_dashed_line(painter, seg[0], seg[1], Stroke::new(stroke_w, color));
            } else {
                painter.line_segment([seg[0], seg[1]], Stroke::new(stroke_w, color));
            }
        }
        let n = pts.len();
        match arrow_style {
            ArrowStyle::None => {}
            ArrowStyle::OpenV => {
                paint_open_v_arrowhead(painter, pts[n - 2], pts[n - 1], Stroke::new(stroke_w, color));
            }
            ArrowStyle::FilledTriangle => {
                paint_arrowhead(painter, pts[n - 2], pts[n - 1], color);
            }
            ArrowStyle::HollowTriangle => {
                paint_hollow_arrowhead(
                    painter,
                    pts[n - 2],
                    pts[n - 1],
                    color,
                    Color32::from_rgb(20, 24, 30),
                );
            }
            ArrowStyle::FilledDiamondStart => {
                paint_filled_diamond(painter, pts[1], pts[0], color);
            }
        }
    }

    fn paint_edge_label(
        &self,
        painter: &egui::Painter,
        edge: &RoutedEdge,
        origin: Pos2,
        selected: bool,
    ) {
        if edge.waypoints.len() < 2 || (edge.guard.is_none() && edge.label.is_none()) {
            return;
        }
        let (color, _, _, _) = Self::edge_style(edge.kind, selected);
        let pts: Vec<Pos2> = edge
            .waypoints
            .iter()
            .map(|w| self.world_to_screen(Pos2::new(w.x, w.y), origin))
            .collect();

        // Pick midpoint of the longest segment so labels avoid crowded stub turns.
        let mut best_mid = pts[pts.len() / 2];
        let mut best_len = -1.0f32;
        for seg in pts.windows(2) {
            let len = seg[0].distance_sq(seg[1]);
            if len > best_len {
                best_len = len;
                best_mid = Pos2::new((seg[0].x + seg[1].x) * 0.5, (seg[0].y + seg[1].y) * 0.5);
            }
        }

        let (txt, fg, border) = if let Some(ref guard) = edge.guard {
            (format!("[{guard}]"), Color32::from_rgb(255, 225, 140), color)
        } else if let Some(ref lbl) = edge.label {
            (lbl.clone(), color, Color32::from_rgb(65, 76, 94))
        } else {
            return;
        };
        let font_sz = (10.5 * self.zoom).clamp(8.5, 13.0);
        let text_pos = best_mid + Vec2::new(0.0, -4.0);
        let bg = Rect::from_center_size(
            text_pos,
            Vec2::new((txt.len() as f32 * font_sz * 0.62 + 10.0).max(28.0), font_sz + 6.0),
        );
        painter.rect_filled(bg, 3.0, Color32::from_rgba_unmultiplied(20, 26, 34, 225));
        painter.rect_stroke(bg, 3.0, Stroke::new(0.9, border), StrokeKind::Inside);
        painter.text(
            text_pos,
            Align2::CENTER_CENTER,
            txt,
            FontId::monospace(font_sz),
            fg,
        );
    }
}

fn project_point_to_border(p: Pos2, rect: Rect2) -> PortPlacement {
    let w = rect.w.max(1.0);
    let h = rect.h.max(1.0);
    let dist_top = (p.y - rect.y).abs();
    let dist_bottom = (p.y - (rect.y + h)).abs();
    let dist_left = (p.x - rect.x).abs();
    let dist_right = (p.x - (rect.x + w)).abs();
    let min_d = dist_top.min(dist_bottom).min(dist_left).min(dist_right);

    if (min_d - dist_top).abs() < 0.01 {
        PortPlacement {
            side: PortSide::Top,
            offset: ((p.x - rect.x) / w).clamp(0.1, 0.9),
        }
    } else if (min_d - dist_bottom).abs() < 0.01 {
        PortPlacement {
            side: PortSide::Bottom,
            offset: ((p.x - rect.x) / w).clamp(0.1, 0.9),
        }
    } else if (min_d - dist_left).abs() < 0.01 {
        PortPlacement {
            side: PortSide::Left,
            offset: ((p.y - rect.y) / h).clamp(0.1, 0.9),
        }
    } else {
        PortPlacement {
            side: PortSide::Right,
            offset: ((p.y - rect.y) / h).clamp(0.1, 0.9),
        }
    }
}

fn infer_unambiguous_link(
    src: NodeShape,
    dst: NodeShape,
    view_kind: ViewKind,
) -> Option<RelationshipKind> {
    if matches!(
        src,
        NodeShape::StartCircle
            | NodeShape::ForkJoinBar
            | NodeShape::MergeDiamond
            | NodeShape::ActionBox
    ) && matches!(
        dst,
        NodeShape::ActionBox
            | NodeShape::ForkJoinBar
            | NodeShape::MergeDiamond
            | NodeShape::DecideDiamond
            | NodeShape::DoneBullseye
    ) && src != NodeShape::DecideDiamond
    {
        return Some(RelationshipKind::Succession);
    }
    if view_kind == ViewKind::StateTransition
        && matches!(src, NodeShape::StateBox | NodeShape::StartCircle)
        && matches!(dst, NodeShape::StateBox | NodeShape::DoneBullseye)
    {
        return Some(RelationshipKind::Transition);
    }
    if view_kind == ViewKind::ActionFlow
        && matches!(
            src,
            NodeShape::StartCircle
                | NodeShape::ActionBox
                | NodeShape::ForkJoinBar
                | NodeShape::MergeDiamond
        )
    {
        return Some(RelationshipKind::Succession);
    }
    if src == NodeShape::PartBlock && dst == NodeShape::RequirementCard {
        return Some(RelationshipKind::Satisfy);
    }
    if src == NodeShape::CaseOval && dst == NodeShape::RequirementCard {
        return Some(RelationshipKind::Verify);
    }
    None
}

fn paint_dashed_line(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    let delta = b - a;
    let len = delta.length();
    if len < 1.0 {
        return;
    }
    let dir = delta / len;
    let mut dist = 0.0;
    let dash = 7.0;
    let gap = 5.0;
    while dist < len {
        let end = (dist + dash).min(len);
        painter.line_segment([a + dir * dist, a + dir * end], stroke);
        dist += dash + gap;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArrowStyle {
    None,
    OpenV,
    FilledTriangle,
    HollowTriangle,
    FilledDiamondStart,
}

fn paint_open_v_arrowhead(painter: &egui::Painter, from: Pos2, tip: Pos2, stroke: Stroke) {
    let dir = (tip - from).normalized();
    if dir.length_sq() < 0.1 {
        return;
    }
    let perp = Vec2::new(-dir.y, dir.x);
    let p1 = tip - dir * 9.5 + perp * 4.5;
    let p2 = tip - dir * 9.5 - perp * 4.5;
    painter.line_segment([tip, p1], stroke);
    painter.line_segment([tip, p2], stroke);
}

fn paint_filled_diamond(painter: &egui::Painter, from: Pos2, tip: Pos2, color: Color32) {
    let dir = (tip - from).normalized();
    if dir.length_sq() < 0.1 {
        return;
    }
    let perp = Vec2::new(-dir.y, dir.x);
    let mid = tip - dir * 5.5;
    let tail = tip - dir * 11.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, mid + perp * 4.0, tail, mid - perp * 4.0],
        color,
        Stroke::NONE,
    ));
}

fn paint_arrowhead(painter: &egui::Painter, from: Pos2, tip: Pos2, color: Color32) {
    let dir = (tip - from).normalized();
    if dir.length_sq() < 0.1 {
        return;
    }
    let perp = Vec2::new(-dir.y, dir.x);
    let p1 = tip - dir * 9.0 + perp * 4.5;
    let p2 = tip - dir * 9.0 - perp * 4.5;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, p1, p2],
        color,
        Stroke::NONE,
    ));
}

fn paint_hollow_arrowhead(
    painter: &egui::Painter,
    from: Pos2,
    tip: Pos2,
    stroke_color: Color32,
    fill_color: Color32,
) {
    let dir = (tip - from).normalized();
    if dir.length_sq() < 0.1 {
        return;
    }
    let perp = Vec2::new(-dir.y, dir.x);
    let p1 = tip - dir * 11.0 + perp * 5.5;
    let p2 = tip - dir * 11.0 - perp * 5.5;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, p1, p2],
        fill_color,
        Stroke::new(1.6, stroke_color),
    ));
}

fn point_segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq < 0.01 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    let proj = a + ab * t;
    p.distance(proj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::SYSML_V2_CATALOG;
    use syssur_layout::Point2;

    fn test_layout() -> (LayoutOutput, BTreeMap<String, NodeMeta>) {
        let id_a = ElementId::new("Pkg::BlockA");
        let id_b = ElementId::new("Pkg::BlockB");
        let path_a = InstancePath::root(id_a.clone());
        let path_b = InstancePath::root(id_b.clone());

        let mut nodes = BTreeMap::new();
        nodes.insert(
            path_a.canonical_key(),
            PositionedNode {
                path: path_a.clone(),
                label: "BlockA".into(),
                shape: NodeShape::PartBlock,
                rect: Rect2 {
                    x: 100.0,
                    y: 100.0,
                    w: 180.0,
                    h: 80.0,
                },
                ports: vec![],
            },
        );
        nodes.insert(
            path_b.canonical_key(),
            PositionedNode {
                path: path_b.clone(),
                label: "BlockB".into(),
                shape: NodeShape::PartBlock,
                rect: Rect2 {
                    x: 420.0,
                    y: 100.0,
                    w: 180.0,
                    h: 80.0,
                },
                ports: vec![],
            },
        );

        let mut meta = BTreeMap::new();
        for (p, td) in [(&path_a, id_a), (&path_b, id_b)] {
            meta.insert(
                p.canonical_key(),
                NodeMeta {
                    is_def: true,
                    type_name: None,
                    type_def: Some(td),
                    specializes: None,
                    multiplicity: None,
                    source_file: "test.sysml".into(),
                    origin: FeatureOrigin::Owned,
                    attributes: vec![],
                    short_id: None,
                    doc: None,
                    stereotype: "«part def»".into(),
                },
            );
        }

        let layout = LayoutOutput {
            nodes,
            edges: vec![],
            bounds: Rect2 {
                x: 0.0,
                y: 0.0,
                w: 800.0,
                h: 600.0,
            },
        };
        (layout, meta)
    }

    fn run_canvas_step(
        ctx: &egui::Context,
        canvas: &mut DiagramCanvas,
        layout: &LayoutOutput,
        meta: &BTreeMap<String, NodeMeta>,
        active_tool: &mut Option<&'static SysmlCatalogItem>,
        events: Vec<egui::Event>,
    ) -> (Option<CanvasAction>, Rect) {
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)));
        raw.events = events;
        let mut action = None;
        let mut avail_rect = Rect::NOTHING;
        let mut palette_search = String::new();
        let mut out = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                avail_rect = ui.available_rect_before_wrap();
                action = canvas.show(
                    ui,
                    layout,
                    meta,
                    &["Pkg".to_string()],
                    ViewKind::General,
                    LayoutDirection::TopToBottom,
                    false,
                    &mut palette_search,
                    active_tool,
                    "test.sysml",
                    Locale::En,
                );
            });
        });
        out.textures_delta.clear();
        (action, avail_rect)
    }

    #[test]
    fn click_with_small_jitter_selects_node_without_moving() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        // Warm-up frame so the canvas rect and spatial state exist.
        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let center = rect.min + Vec2::new(190.0, 140.0);

        let mut actions = Vec::new();
        for evs in [
            vec![
                egui::Event::PointerMoved(center),
                egui::Event::PointerButton {
                    pos: center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(center + Vec2::new(2.0, 1.0))],
            vec![egui::Event::PointerButton {
                pos: center + Vec2::new(2.0, 1.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let (act, _) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, evs);
            if let Some(a) = act {
                actions.push(a);
            }
        }

        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, CanvasAction::MoveNodes { .. })),
            "2px click jitter must not emit MoveNodes, got: {actions:?}"
        );
        assert_eq!(
            canvas.selected.as_ref().map(InstancePath::canonical_key),
            Some("Pkg::BlockA".to_string())
        );
    }

    #[test]
    fn drag_past_threshold_moves_node() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let start = rect.min + Vec2::new(190.0, 140.0);
        let end = start + Vec2::new(50.0, 30.0);

        let mut total_dx = 0.0_f32;
        let mut total_dy = 0.0_f32;
        for evs in [
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(end)],
            vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ] {
            let (act, _) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, evs);
            if let Some(CanvasAction::MoveNodes { deltas }) = act {
                for (_, dx, dy) in deltas {
                    total_dx += dx;
                    total_dy += dy;
                }
            }
        }

        assert!(
            total_dx > 40.0 && total_dy > 20.0,
            "expected positive MoveNodes delta across drag threshold, got ({total_dx}, {total_dy})"
        );
    }

    #[test]
    fn right_click_jitter_does_not_drag_or_pan_and_records_world_pos() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::new(25.0, 30.0);
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let target = rect.min + Vec2::new(200.0, 160.0);
        let expected_world = canvas.screen_to_world(target + Vec2::new(3.0, 2.0), rect.min);

        let mut moved = false;
        for evs in [
            vec![
                egui::Event::PointerMoved(target),
                egui::Event::PointerButton {
                    pos: target,
                    button: egui::PointerButton::Secondary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![
                egui::Event::PointerMoved(target + Vec2::new(3.0, 2.0)),
                egui::Event::PointerButton {
                    pos: target + Vec2::new(3.0, 2.0),
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        ] {
            let (act, _) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, evs);
            if matches!(act, Some(CanvasAction::MoveNodes { .. })) {
                moved = true;
            }
        }

        assert!(!moved, "right-click jitter must never move nodes");
        assert_eq!(canvas.pan, Vec2::new(25.0, 30.0));
        let recorded = canvas
            .context_world_pos
            .expect("right-click should record context_world_pos");
        assert!(
            (recorded.x - expected_world.x).abs() < 1.0
                && (recorded.y - expected_world.y).abs() < 1.0,
            "expected world {expected_world:?}, got {recorded:?}"
        );
    }

    #[test]
    fn active_palette_tool_click_places_at_cursor_world_pos() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::new(40.0, 20.0);
        canvas.zoom = 1.25;
        canvas.rebuild_spatial_index(&layout);
        let part_def = SYSML_V2_CATALOG
            .iter()
            .find(|i| i.id == "part_def")
            .expect("part_def item");
        let mut tool = Some(part_def);

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let click_pt = rect.min + Vec2::new(500.0, 400.0);
        let expected_world = canvas.screen_to_world(click_pt, rect.min);

        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(click_pt),
                egui::Event::PointerButton {
                    pos: click_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (act, _) = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: click_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        match act {
            Some(CanvasAction::PlaceCatalogItem {
                item,
                world_pos: Some(Point2 { x, y }),
                ..
            }) => {
                assert_eq!(item.id, "part_def");
                assert!(
                    (x - expected_world.x).abs() < 0.5 && (y - expected_world.y).abs() < 0.5,
                    "expected world {expected_world:?}, got ({x}, {y})"
                );
            }
            other => panic!("expected PlaceCatalogItem at cursor, got {other:?}"),
        }
        assert!(tool.is_none(), "active tool should disarm after click place");
    }

    #[test]
    fn scroll_zoom_keeps_world_point_under_cursor_anchored() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::new(60.0, 60.0);
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let cursor = rect.min + Vec2::new(420.0, 310.0);
        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerMoved(cursor)],
        );
        let world_before = canvas.screen_to_world(cursor, rect.min);

        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, 120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let world_after = canvas.screen_to_world(cursor, rect.min);

        assert!(
            canvas.zoom > 1.0,
            "expected scroll wheel up to increase zoom, got {}",
            canvas.zoom
        );
        assert!(
            (world_before.x - world_after.x).abs() < 0.5
                && (world_before.y - world_after.y).abs() < 0.5,
            "expected cursor-anchored world point {world_before:?}, got {world_after:?}"
        );
    }

    #[test]
    fn palette_item_click_arms_active_tool() {
        let ctx = egui::Context::default();
        let mut search = "Part Definition".to_string();
        let mut active_tool = None;
        let mut click_center = Pos2::ZERO;

        fn find_palette_text(shape: &egui::epaint::Shape) -> Option<Pos2> {
            match shape {
                egui::epaint::Shape::Text(t)
                    if t.galley.text() == "Part Definition" && t.pos.y > 60.0 =>
                {
                    Some(t.pos + Vec2::new(30.0, 6.0))
                }
                egui::epaint::Shape::Vec(v) => v.iter().find_map(find_palette_text),
                _ => None,
            }
        }

        let run_palette = |ctx: &egui::Context,
                           search: &mut String,
                           active_tool: &mut Option<&'static SysmlCatalogItem>,
                           events: Vec<egui::Event>|
         -> Pos2 {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)));
            raw.events = events;
            let mut center = Pos2::ZERO;
            let mut out = ctx.run_ui(raw, |ui| {
                egui::Panel::left("pal_test").show(ui, |ui| {
                    show_optional_palette(ui, search, active_tool);
                });
            });
            out.textures_delta.clear();
            for prim in &out.shapes {
                if let Some(p) = find_palette_text(&prim.shape) {
                    center = p;
                }
            }
            center
        };

        // Frame 1: find Part Definition label center
        let c = run_palette(&ctx, &mut search, &mut active_tool, vec![]);
        if c != Pos2::ZERO {
            click_center = c;
        }
        assert_ne!(click_center, Pos2::ZERO, "Part Definition item must render");

        let _ = run_palette(
            &ctx,
            &mut search,
            &mut active_tool,
            vec![
                egui::Event::PointerMoved(click_center),
                egui::Event::PointerButton {
                    pos: click_center,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let _ = run_palette(
            &ctx,
            &mut search,
            &mut active_tool,
            vec![egui::Event::PointerButton {
                pos: click_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert_eq!(
            active_tool.map(|i| i.id),
            Some("part_def"),
            "clicking palette item must arm active tool even when draggable"
        );
    }

    #[test]
    fn dragging_corner_handle_resizes_selected_block() {
        let ctx = egui::Context::default();
        let (layout, meta) = test_layout();
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.selected = Some(InstancePath::root(ElementId::new("Pkg::BlockA")));
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        // BlockA rect is (100, 100, 180, 80) -> bottom-right corner at (280, 180)
        let corner_start = rect.min + Vec2::new(280.0, 180.0);
        let drag_end = corner_start + Vec2::new(60.0, 40.0);

        let mut resize_action = None;
        for evs in [
            vec![
                egui::Event::PointerMoved(corner_start),
                egui::Event::PointerButton {
                    pos: corner_start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            vec![egui::Event::PointerMoved(drag_end)],
        ] {
            let (act, _) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, evs);
            if matches!(act, Some(CanvasAction::ResizeNode { .. })) {
                resize_action = act;
            }
        }

        match resize_action {
            Some(CanvasAction::ResizeNode {
                node_key,
                width,
                height,
                anchor_x,
                anchor_y,
                anchor_right,
                anchor_bottom,
            }) => {
                assert_eq!(node_key, "Pkg::BlockA");
                assert!((width - 240.0).abs() < 1.0, "expected width ~240, got {width}");
                assert!((height - 120.0).abs() < 1.0, "expected height ~120, got {height}");
                assert!((anchor_x - 100.0).abs() < 0.5 && (anchor_y - 100.0).abs() < 0.5);
                assert!(!anchor_right && !anchor_bottom);
            }
            other => panic!("expected ResizeNode action, got {other:?}"),
        }
    }

    #[test]
    fn clicking_port_selects_port_instead_of_parent_node() {
        let ctx = egui::Context::default();
        let (mut layout, meta) = test_layout();
        let node_a = layout
            .nodes
            .get_mut("Pkg::BlockA")
            .expect("BlockA test node");
        node_a.ports.push(PositionedPort {
            name: "pwrOut".into(),
            direction: PortDirection::Out,
            is_conjugated: false,
            type_label: Some("PowerPort".into()),
            side: PortSide::Right,
            offset: 0.5,
            anchor: Point2 { x: 280.0, y: 140.0 },
            normal: Point2 { x: 1.0, y: 0.0 },
        });
        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let port_pt = rect.min + Vec2::new(280.0, 140.0);

        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(port_pt),
                egui::Event::PointerButton {
                    pos: port_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (act, _) = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: port_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        match act {
            Some(CanvasAction::SelectPort {
                node_path,
                port_name,
                ..
            }) => {
                assert_eq!(node_path.canonical_key(), "Pkg::BlockA");
                assert_eq!(port_name, "pwrOut");
            }
            other => panic!("expected SelectPort action, got {other:?}"),
        }
        assert_eq!(
            canvas.selected_port,
            Some(("Pkg::BlockA".to_string(), "pwrOut".to_string()))
        );
        assert!(canvas.selected.is_none());
    }

    #[test]
    fn clicking_connector_inside_compound_container_selects_edge() {
        let ctx = egui::Context::default();
        let parent_id = ElementId::new("Pkg::System");
        let child_a = ElementId::new("Pkg::System::a");
        let child_b = ElementId::new("Pkg::System::b");
        let path_parent = InstancePath::root(parent_id.clone());
        let path_a = path_parent.push(child_a.clone());
        let path_b = path_parent.push(child_b.clone());

        let mut nodes = BTreeMap::new();
        nodes.insert(
            path_parent.canonical_key(),
            PositionedNode {
                path: path_parent.clone(),
                label: "System".into(),
                shape: NodeShape::PartBlock,
                rect: Rect2 {
                    x: 40.0,
                    y: 40.0,
                    w: 500.0,
                    h: 240.0,
                },
                ports: vec![],
            },
        );
        nodes.insert(
            path_a.canonical_key(),
            PositionedNode {
                path: path_a.clone(),
                label: "a".into(),
                shape: NodeShape::PartBlock,
                rect: Rect2 {
                    x: 70.0,
                    y: 110.0,
                    w: 140.0,
                    h: 80.0,
                },
                ports: vec![],
            },
        );
        nodes.insert(
            path_b.canonical_key(),
            PositionedNode {
                path: path_b.clone(),
                label: "b".into(),
                shape: NodeShape::PartBlock,
                rect: Rect2 {
                    x: 360.0,
                    y: 110.0,
                    w: 140.0,
                    h: 80.0,
                },
                ports: vec![],
            },
        );

        let mut meta = BTreeMap::new();
        for (p, td, is_def) in [
            (&path_parent, parent_id, true),
            (&path_a, child_a, false),
            (&path_b, child_b, false),
        ] {
            meta.insert(
                p.canonical_key(),
                NodeMeta {
                    is_def,
                    type_name: None,
                    type_def: Some(td),
                    specializes: None,
                    multiplicity: None,
                    source_file: "test.sysml".into(),
                    origin: FeatureOrigin::Owned,
                    attributes: vec![],
                    short_id: None,
                    doc: None,
                    stereotype: if is_def { "«part def»" } else { "«part»" }.into(),
                },
            );
        }

        let layout = LayoutOutput {
            nodes,
            edges: vec![RoutedEdge {
                id: "Pkg::System::link#0".into(),
                kind: RelationshipKind::Connection,
                label: Some("link : PowerInterface".into()),
                guard: None,
                waypoints: vec![
                    Point2 { x: 210.0, y: 150.0 },
                    Point2 { x: 360.0, y: 150.0 },
                ],
            }],
            bounds: Rect2 {
                x: 0.0,
                y: 0.0,
                w: 600.0,
                h: 340.0,
            },
        };

        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let edge_pt = rect.min + Vec2::new(285.0, 150.0);

        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(edge_pt),
                egui::Event::PointerButton {
                    pos: edge_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (act, _) = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: edge_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        assert!(
            matches!(act, Some(CanvasAction::SelectEdge { index: 0 })),
            "expected SelectEdge {{ index: 0 }} inside compound container, got {act:?}"
        );
        assert_eq!(canvas.selected_edge, Some(0));
    }

    #[test]
    fn palette_hides_attribute_usage() {
        let ctx = egui::Context::default();
        let mut search = "attribute".to_string();
        let mut active_tool = None;

        fn has_text(shape: &egui::epaint::Shape, needle: &str) -> bool {
            match shape {
                egui::epaint::Shape::Text(t) => t.galley.text() == needle,
                egui::epaint::Shape::Vec(v) => v.iter().any(|s| has_text(s, needle)),
                _ => false,
            }
        }

        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)));
        let mut out = ctx.run_ui(raw, |ui| {
            egui::Panel::left("pal_curated_test").show(ui, |ui| {
                show_optional_palette(ui, &mut search, &mut active_tool);
            });
        });
        out.textures_delta.clear();

        let rendered_attr = out
            .shapes
            .iter()
            .any(|p| has_text(&p.shape, "Attribute Usage"));
        assert!(
            !rendered_attr,
            "Attribute Usage must be hidden from Diagram Palette"
        );
    }

    #[test]
    fn relationship_tool_clicks_connect_two_ports() {
        let ctx = egui::Context::default();
        let (mut layout, meta) = test_layout();
        layout
            .nodes
            .get_mut("Pkg::BlockA")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrOut".into(),
                direction: PortDirection::Out,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Right,
                offset: 0.5,
                anchor: Point2 { x: 280.0, y: 140.0 },
                normal: Point2 { x: 1.0, y: 0.0 },
            });
        layout
            .nodes
            .get_mut("Pkg::BlockB")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrIn".into(),
                direction: PortDirection::In,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Left,
                offset: 0.5,
                anchor: Point2 { x: 360.0, y: 140.0 },
                normal: Point2 { x: -1.0, y: 0.0 },
            });

        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);

        let conn_tool = SYSML_V2_CATALOG
            .iter()
            .find(|i| i.id == "rel_connection")
            .expect("rel_connection tool");
        let mut tool = Some(conn_tool);

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let p_a = rect.min + Vec2::new(280.0, 140.0);
        let p_b = rect.min + Vec2::new(360.0, 140.0);

        // Click source port on BlockA
        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(p_a),
                egui::Event::PointerButton {
                    pos: p_a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: p_a,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            canvas.wire_armed_port.is_some(),
            "first click on port with relationship tool should arm port wiring"
        );

        // Click target port on BlockB
        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(p_b),
                egui::Event::PointerButton {
                    pos: p_b,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        let (act, _) = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: p_b,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        match act {
            Some(CanvasAction::ConnectPorts {
                source,
                target,
                rel_keyword,
            }) => {
                assert_eq!(source.port_name, "pwrOut");
                assert_eq!(target.port_name, "pwrIn");
                assert_eq!(rel_keyword.as_deref(), Some("connection"));
            }
            other => panic!("expected ConnectPorts from relationship clicks, got {other:?}"),
        }
        assert!(tool.is_none(), "relationship tool disarms after port connect");
    }

    #[test]
    fn shift_drag_between_ports_emits_connect_ports() {
        let ctx = egui::Context::default();
        let (mut layout, meta) = test_layout();
        layout
            .nodes
            .get_mut("Pkg::BlockA")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrOut".into(),
                direction: PortDirection::Out,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Right,
                offset: 0.5,
                anchor: Point2 { x: 280.0, y: 140.0 },
                normal: Point2 { x: 1.0, y: 0.0 },
            });
        layout
            .nodes
            .get_mut("Pkg::BlockB")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrIn".into(),
                direction: PortDirection::In,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Left,
                offset: 0.5,
                anchor: Point2 { x: 360.0, y: 140.0 },
                normal: Point2 { x: -1.0, y: 0.0 },
            });

        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let mut tool = None;

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let p_a = rect.min + Vec2::new(280.0, 140.0);
        let p_b = rect.min + Vec2::new(360.0, 140.0);

        let mut run_shift = |events: Vec<egui::Event>, canvas: &mut DiagramCanvas| -> Option<CanvasAction> {
            let mut raw = egui::RawInput::default();
            raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0)));
            raw.events = events;
            let mut action = None;
            let mut palette_search = String::new();
            let mut out = ctx.run_ui(raw, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    action = canvas.show(
                        ui,
                        &layout,
                        &meta,
                        &["Pkg".to_string()],
                        ViewKind::General,
                        LayoutDirection::TopToBottom,
                        false,
                        &mut palette_search,
                        &mut tool,
                        "test.sysml",
                        Locale::En,
                    );
                });
            });
            out.textures_delta.clear();
            action
        };

        let _ = run_shift(
            vec![
                egui::Event::ModifiersChanged(egui::Modifiers::SHIFT),
                egui::Event::PointerMoved(p_a),
                egui::Event::PointerButton {
                    pos: p_a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::SHIFT,
                },
                egui::Event::PointerMoved(p_a + Vec2::new(10.0, 0.0)),
            ],
            &mut canvas,
        );
        let _ = run_shift(vec![egui::Event::PointerMoved(p_b)], &mut canvas);
        let act = run_shift(
            vec![egui::Event::PointerButton {
                pos: p_b,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::SHIFT,
            }],
            &mut canvas,
        );

        match act {
            Some(CanvasAction::ConnectPorts { source, target, .. }) => {
                assert_eq!(source.port_name, "pwrOut");
                assert_eq!(target.port_name, "pwrIn");
            }
            other => panic!("expected ConnectPorts from Shift+Drag, got {other:?}"),
        }
    }

    #[test]
    fn relationship_tool_drag_between_ports_emits_connect_ports() {
        let ctx = egui::Context::default();
        let (mut layout, meta) = test_layout();
        layout
            .nodes
            .get_mut("Pkg::BlockA")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrOut".into(),
                direction: PortDirection::Out,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Right,
                offset: 0.5,
                anchor: Point2 { x: 280.0, y: 140.0 },
                normal: Point2 { x: 1.0, y: 0.0 },
            });
        layout
            .nodes
            .get_mut("Pkg::BlockB")
            .unwrap()
            .ports
            .push(PositionedPort {
                name: "pwrIn".into(),
                direction: PortDirection::In,
                is_conjugated: false,
                type_label: Some("PowerPort".into()),
                side: PortSide::Left,
                offset: 0.5,
                anchor: Point2 { x: 360.0, y: 140.0 },
                normal: Point2 { x: -1.0, y: 0.0 },
            });

        let mut canvas = DiagramCanvas::default();
        canvas.pan = Vec2::ZERO;
        canvas.zoom = 1.0;
        canvas.rebuild_spatial_index(&layout);
        let flow_tool = SYSML_V2_CATALOG
            .iter()
            .find(|i| i.id == "rel_flow")
            .expect("rel_flow tool");
        let mut tool = Some(flow_tool);

        let (_, rect) = run_canvas_step(&ctx, &mut canvas, &layout, &meta, &mut tool, vec![]);
        let p_a = rect.min + Vec2::new(280.0, 140.0);
        let p_b = rect.min + Vec2::new(360.0, 140.0);

        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![
                egui::Event::PointerMoved(p_a),
                egui::Event::PointerButton {
                    pos: p_a,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerMoved(p_a + Vec2::new(12.0, 0.0)),
            ],
        );
        let _ = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerMoved(p_b)],
        );
        let (act, _) = run_canvas_step(
            &ctx,
            &mut canvas,
            &layout,
            &meta,
            &mut tool,
            vec![egui::Event::PointerButton {
                pos: p_b,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        match act {
            Some(CanvasAction::ConnectPorts {
                source,
                target,
                rel_keyword,
            }) => {
                assert_eq!(source.port_name, "pwrOut");
                assert_eq!(target.port_name, "pwrIn");
                assert_eq!(rel_keyword.as_deref(), Some("flow"));
            }
            other => panic!("expected ConnectPorts from relationship tool drag, got {other:?}"),
        }
        assert!(tool.is_none(), "relationship tool disarms after drag connect");
    }
}
