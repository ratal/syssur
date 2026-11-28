//! Deterministic diagram auto-layout for Syssur:
//! 1. Canonical AST-span pre-sorter so Dagre inputs are 100% order-independent.
//! 2. Dagre compound layout for node/container bounding boxes.
//! 3. Border port pinning and orthogonal A*/channel routing between ports.

use dagre::graph::{Graph, GraphOptions};
use dagre::layout;
use dagre::layout::types::{EdgeLabel, LayoutOptions, NodeLabel, RankDir};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use syssur_model::{InstancePath, PortDirection, RelationshipKind, Span};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutDirection {
    #[default]
    LeftToRight,
    TopToBottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeShape {
    #[default]
    PartBlock,
    ActionBox,
    StartCircle,
    DoneBullseye,
    ForkJoinBar,
    DecideDiamond,
    MergeDiamond,
    RequirementCard,
    StateBox,
    ConstraintCard,
    CaseOval,
    GenericElement,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect2 {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect2 {
    pub fn contains_padded(&self, p: Point2, pad: f32) -> bool {
        p.x >= self.x - pad
            && p.x <= self.x + self.w + pad
            && p.y >= self.y - pad
            && p.y <= self.y + self.h + pad
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutPortSpec {
    pub name: String,
    pub direction: PortDirection,
    pub is_conjugated: bool,
    pub type_label: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNodeSpec {
    pub path: InstancePath,
    pub parent: Option<InstancePath>,
    pub label: String,
    pub shape: NodeShape,
    pub width: f32,
    pub height: f32,
    pub ports: Vec<LayoutPortSpec>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdgeSpec {
    pub id: String,
    pub source_node: InstancePath,
    pub source_port: Option<String>,
    pub target_node: InstancePath,
    pub target_port: Option<String>,
    pub kind: RelationshipKind,
    pub label: Option<String>,
    pub guard: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutInput {
    pub direction: LayoutDirection,
    pub nodes: Vec<LayoutNodeSpec>,
    pub edges: Vec<LayoutEdgeSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortSide {
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PortPlacement {
    pub side: PortSide,
    pub offset: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionedPort {
    pub name: String,
    pub direction: PortDirection,
    pub is_conjugated: bool,
    pub type_label: Option<String>,
    pub side: PortSide,
    pub offset: f32,
    pub anchor: Point2,
    pub normal: Point2,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionedNode {
    pub path: InstancePath,
    pub label: String,
    pub shape: NodeShape,
    pub rect: Rect2,
    pub ports: Vec<PositionedPort>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutedEdge {
    pub id: String,
    pub kind: RelationshipKind,
    pub label: Option<String>,
    pub guard: Option<String>,
    pub waypoints: Vec<Point2>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayoutOutput {
    pub nodes: BTreeMap<String, PositionedNode>,
    pub edges: Vec<RoutedEdge>,
    pub bounds: Rect2,
}

impl Default for Rect2 {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        }
    }
}

/// Computes a 100% deterministic layout from `LayoutInput`.
pub fn compute_deterministic_layout(input: &LayoutInput) -> LayoutOutput {
    compute_layout_with_overrides(input, &BTreeMap::new())
}

/// Computes a deterministic baseline layout from `LayoutInput`, then applies optional
/// per-node `(dx, dy)` drag overrides (expanding parent containers and re-routing all
/// connected edges orthogonally via A* around the moved node boxes).
pub fn compute_layout_with_overrides(
    input: &LayoutInput,
    node_offsets: &BTreeMap<String, Point2>,
) -> LayoutOutput {
    compute_layout_with_port_overrides(input, node_offsets, &BTreeMap::new())
}

/// Computes deterministic baseline + node drag offsets + interactive per-port border placement
/// overrides (`(node_canonical_key, port_name) -> PortPlacement`), then routes all edges
/// orthogonally around every non-enclosing node obstacle.
pub fn compute_layout_with_port_overrides(
    input: &LayoutInput,
    node_offsets: &BTreeMap<String, Point2>,
    port_overrides: &BTreeMap<(String, String), PortPlacement>,
) -> LayoutOutput {
    if input.nodes.is_empty() {
        return LayoutOutput::default();
    }

    // 1. Canonical AST-span pre-sort for nodes and edges.
    let mut nodes = input.nodes.clone();
    nodes.sort_by(|a, b| {
        a.span
            .cmp(&b.span)
            .then_with(|| a.path.canonical_key().cmp(&b.path.canonical_key()))
    });

    let mut edges = input.edges.clone();
    edges.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.id.cmp(&b.id)));

    // 2. Build compound Dagre graph in sorted order.
    let has_compound = nodes.iter().any(|n| n.parent.is_some());
    let parent_keys: BTreeSet<String> = nodes
        .iter()
        .filter_map(|n| n.parent.as_ref().map(|p| p.canonical_key()))
        .collect();
    let mut graph: Graph<NodeLabel, EdgeLabel> = Graph::with_options(GraphOptions {
        directed: true,
        multigraph: true,
        compound: has_compound,
    });

    for node in &nodes {
        let key = node.path.canonical_key();
        let is_parent = parent_keys.contains(&key);
        graph.set_node(
            key.clone(),
            Some(NodeLabel {
                width: node.width as f64,
                height: node.height as f64,
                padding: if is_parent { 32.0 } else { 20.0 },
                padding_x: is_parent.then_some(26.0),
                padding_y: is_parent.then_some(44.0),
                ..Default::default()
            }),
        );
    }

    if has_compound {
        for node in &nodes {
            if let Some(ref parent) = node.parent {
                let pkey = parent.canonical_key();
                if graph.has_node(&pkey) {
                    graph.set_parent(&node.path.canonical_key(), Some(&pkey));
                }
            }
        }
    }

    for edge in &edges {
        let src = edge.source_node.canonical_key();
        let dst = edge.target_node.canonical_key();
        if graph.has_node(&src) && graph.has_node(&dst) {
            graph.set_edge(
                src,
                dst,
                Some(EdgeLabel {
                    minlen: 1,
                    weight: 1,
                    ..Default::default()
                }),
                Some(&edge.id),
            );
        }
    }

    let rankdir = match input.direction {
        LayoutDirection::LeftToRight => RankDir::LR,
        LayoutDirection::TopToBottom => RankDir::TB,
    };

    layout(
        &mut graph,
        Some(LayoutOptions {
            rankdir,
            nodesep: 48.0,
            ranksep: 68.0,
            edgesep: 24.0,
            marginx: 32.0,
            marginy: 32.0,
            ..Default::default()
        }),
    );

    // 3. Extract base bounding boxes & apply interactive node offsets.
    let mut positioned_nodes = BTreeMap::new();
    for node in &nodes {
        let key = node.path.canonical_key();
        let label = graph.node(&key);
        let (cx, cy, w, h) = match label {
            Some(nl) => (
                nl.x.unwrap_or(0.0) as f32,
                nl.y.unwrap_or(0.0) as f32,
                nl.width.max(node.width as f64) as f32,
                nl.height.max(node.height as f64) as f32,
            ),
            None => (0.0, 0.0, node.width, node.height),
        };
        let off = node_offsets
            .get(&key)
            .copied()
            .unwrap_or(Point2 { x: 0.0, y: 0.0 });
        let rect = Rect2 {
            x: cx - w * 0.5 + off.x,
            y: cy - h * 0.5 + off.y,
            w,
            h,
        };

        positioned_nodes.insert(
            key,
            PositionedNode {
                path: node.path.clone(),
                label: node.label.clone(),
                shape: node.shape,
                rect,
                ports: Vec::new(),
            },
        );
    }

    // Expand compound parent containers bottom-up so multi-level parents enclose all children
    // with header clearance even on initial auto-layout and after interactive node drags.
    if has_compound {
        let mut child_order: Vec<usize> = (0..nodes.len()).collect();
        child_order.sort_by(|&a, &b| {
            nodes[b]
                .path
                .0
                .len()
                .cmp(&nodes[a].path.0.len())
                .then_with(|| {
                    nodes[a]
                        .path
                        .canonical_key()
                        .cmp(&nodes[b].path.canonical_key())
                })
        });
        for idx in child_order {
            let node = &nodes[idx];
            if let Some(ref parent) = node.parent {
                let ckey = node.path.canonical_key();
                let pkey = parent.canonical_key();
                if let (Some(child_rect), Some(parent_node)) = (
                    positioned_nodes.get(&ckey).map(|n| n.rect),
                    positioned_nodes.get_mut(&pkey),
                ) {
                    let pad_x = 22.0;
                    let pad_top = 44.0;
                    let pad_bot = 20.0;
                    let min_x = parent_node.rect.x.min(child_rect.x - pad_x);
                    let min_y = parent_node.rect.y.min(child_rect.y - pad_top);
                    let max_x = (parent_node.rect.x + parent_node.rect.w)
                        .max(child_rect.x + child_rect.w + pad_x);
                    let max_y = (parent_node.rect.y + parent_node.rect.h)
                        .max(child_rect.y + child_rect.h + pad_bot);
                    parent_node.rect = Rect2 {
                        x: min_x,
                        y: min_y,
                        w: max_x - min_x,
                        h: max_y - min_y,
                    };
                }
            }
        }
    }

    let mut obstacles = Vec::with_capacity(nodes.len());
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;

    // Collect read-only centers so smart port pinning can orient ports toward connected neighbors.
    let node_centers: BTreeMap<String, Point2> = positioned_nodes
        .iter()
        .map(|(k, pn)| {
            (
                k.clone(),
                Point2 {
                    x: pn.rect.x + pn.rect.w * 0.5,
                    y: pn.rect.y + pn.rect.h * 0.5,
                },
            )
        })
        .collect();

    for node in &nodes {
        let key = node.path.canonical_key();
        if let Some(pnode) = positioned_nodes.get_mut(&key) {
            pnode.ports = pin_ports_on_border(
                &key,
                node.shape,
                &node.ports,
                pnode.rect,
                &edges,
                &node_centers,
                input.direction,
                port_overrides,
            );
            min_x = min_x.min(pnode.rect.x);
            min_y = min_y.min(pnode.rect.y);
            max_x = max_x.max(pnode.rect.x + pnode.rect.w);
            max_y = max_y.max(pnode.rect.y + pnode.rect.h);
            obstacles.push((key, pnode.rect));
        }
    }

    let global_bounds = if min_x < max_x && min_y < max_y {
        Rect2 {
            x: min_x,
            y: min_y,
            w: max_x - min_x,
            h: max_y - min_y,
        }
    } else {
        Rect2::default()
    };

    // 4. Route edges orthogonally between pinned port anchors around block obstacles.
    let mut routed_edges = Vec::with_capacity(edges.len());
    for (idx, edge) in edges.iter().enumerate() {
        let src_key = edge.source_node.canonical_key();
        let dst_key = edge.target_node.canonical_key();
        let (Some(src_node), Some(dst_node)) =
            (positioned_nodes.get(&src_key), positioned_nodes.get(&dst_key))
        else {
            continue;
        };

        let ((start, start_dir), (end, end_dir)) = resolve_edge_anchors(
            src_node,
            edge.source_port.as_deref(),
            dst_node,
            edge.target_port.as_deref(),
            input.direction,
        );

        // Keep `src_key` and `dst_key` as active obstacles so paths wrap around them cleanly
        // outside their port stubs; exclude enclosing/ancestor compound containers.
        let mut excluded_keys = BTreeSet::new();
        for (okey, orect) in &obstacles {
            if okey != &src_key
                && okey != &dst_key
                && (src_key.starts_with(&format!("{okey}/"))
                    || dst_key.starts_with(&format!("{okey}/"))
                    || rect_encloses(*orect, src_node.rect)
                    || rect_encloses(*orect, dst_node.rect))
            {
                excluded_keys.insert(okey.clone());
            }
        }

        let lane_offset = ((idx % 5) as f32 - 2.0) * 8.0;
        let waypoints = route_orthogonal_astar(
            start,
            start_dir,
            end,
            end_dir,
            &obstacles,
            &excluded_keys,
            global_bounds,
            lane_offset,
        );

        routed_edges.push(RoutedEdge {
            id: edge.id.clone(),
            kind: edge.kind,
            label: edge.label.clone(),
            guard: edge.guard.clone(),
            waypoints,
        });
    }

    let bounds = if min_x < max_x && min_y < max_y {
        Rect2 {
            x: min_x - 32.0,
            y: min_y - 32.0,
            w: (max_x - min_x) + 64.0,
            h: (max_y - min_y) + 64.0,
        }
    } else {
        Rect2::default()
    };

    LayoutOutput {
        nodes: positioned_nodes,
        edges: routed_edges,
        bounds,
    }
}

fn pin_ports_on_border(
    node_key: &str,
    shape: NodeShape,
    specs: &[LayoutPortSpec],
    rect: Rect2,
    edges: &[LayoutEdgeSpec],
    node_centers: &BTreeMap<String, Point2>,
    direction: LayoutDirection,
    port_overrides: &BTreeMap<(String, String), PortPlacement>,
) -> Vec<PositionedPort> {
    let mut sorted = specs.to_vec();
    sorted.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.name.cmp(&b.name)));

    let self_center = node_centers.get(node_key).copied().unwrap_or(Point2 {
        x: rect.x + rect.w * 0.5,
        y: rect.y + rect.h * 0.5,
    });

    // Collect either user-overridden placement or smart automatic side per port.
    let mut fixed_ports: Vec<(LayoutPortSpec, PortPlacement)> = Vec::new();
    let mut auto_top = Vec::new();
    let mut auto_right = Vec::new();
    let mut auto_bottom = Vec::new();
    let mut auto_left = Vec::new();
    let mut inout_idx = 0usize;

    for p in sorted {
        if let Some(&ovr) = port_overrides.get(&(node_key.to_string(), p.name.clone())) {
            fixed_ports.push((
                p,
                PortPlacement {
                    side: ovr.side,
                    offset: ovr.offset.clamp(0.1, 0.9),
                },
            ));
            continue;
        }

        // Check connected neighbors for this specific port first.
        let mut sum_dx = 0.0f32;
        let mut sum_dy = 0.0f32;
        let mut conn_count = 0usize;
        for e in edges {
            let src_k = e.source_node.canonical_key();
            let dst_k = e.target_node.canonical_key();
            if src_k == node_key && e.source_port.as_deref() == Some(p.name.as_str()) {
                if let Some(other) = node_centers.get(&dst_k) {
                    sum_dx += other.x - self_center.x;
                    sum_dy += other.y - self_center.y;
                    conn_count += 1;
                }
            } else if dst_k == node_key && e.target_port.as_deref() == Some(p.name.as_str()) {
                if let Some(other) = node_centers.get(&src_k) {
                    sum_dx += other.x - self_center.x;
                    sum_dy += other.y - self_center.y;
                    conn_count += 1;
                }
            }
        }

        let side = if shape == NodeShape::ActionBox && direction == LayoutDirection::TopToBottom {
            match p.direction.effective_direction(p.is_conjugated) {
                PortDirection::In => PortSide::Left,
                PortDirection::Out => PortSide::Right,
                PortDirection::InOut => {
                    let s = if inout_idx % 2 == 0 {
                        PortSide::Left
                    } else {
                        PortSide::Right
                    };
                    inout_idx += 1;
                    s
                }
            }
        } else if conn_count > 0 && (sum_dx.abs() > 1.0 || sum_dy.abs() > 1.0) {
            if sum_dx.abs() >= sum_dy.abs() {
                if sum_dx >= 0.0 {
                    PortSide::Right
                } else {
                    PortSide::Left
                }
            } else if sum_dy >= 0.0 {
                PortSide::Bottom
            } else {
                PortSide::Top
            }
        } else {
            match (p.direction.effective_direction(p.is_conjugated), direction) {
                (PortDirection::In, LayoutDirection::LeftToRight) => PortSide::Left,
                (PortDirection::Out, LayoutDirection::LeftToRight) => PortSide::Right,
                (PortDirection::In, LayoutDirection::TopToBottom) => PortSide::Top,
                (PortDirection::Out, LayoutDirection::TopToBottom) => PortSide::Bottom,
                (PortDirection::InOut, LayoutDirection::LeftToRight) => {
                    let s = if inout_idx % 2 == 0 {
                        PortSide::Left
                    } else {
                        PortSide::Right
                    };
                    inout_idx += 1;
                    s
                }
                (PortDirection::InOut, LayoutDirection::TopToBottom) => {
                    let sides = [
                        PortSide::Left,
                        PortSide::Right,
                        PortSide::Top,
                        PortSide::Bottom,
                    ];
                    let s = sides[inout_idx % sides.len()];
                    inout_idx += 1;
                    s
                }
            }
        };

        match side {
            PortSide::Top => auto_top.push(p),
            PortSide::Right => auto_right.push(p),
            PortSide::Bottom => auto_bottom.push(p),
            PortSide::Left => auto_left.push(p),
        }
    }

    let make_port = |p: &LayoutPortSpec, side: PortSide, t: f32| -> PositionedPort {
        let clamped = t.clamp(0.08, 0.92);
        let (anchor, normal) = match side {
            PortSide::Top => (
                Point2 {
                    x: rect.x + rect.w * clamped,
                    y: rect.y,
                },
                Point2 { x: 0.0, y: -1.0 },
            ),
            PortSide::Right => (
                Point2 {
                    x: rect.x + rect.w,
                    y: rect.y + rect.h * clamped,
                },
                Point2 { x: 1.0, y: 0.0 },
            ),
            PortSide::Bottom => (
                Point2 {
                    x: rect.x + rect.w * clamped,
                    y: rect.y + rect.h,
                },
                Point2 { x: 0.0, y: 1.0 },
            ),
            PortSide::Left => (
                Point2 {
                    x: rect.x,
                    y: rect.y + rect.h * clamped,
                },
                Point2 { x: -1.0, y: 0.0 },
            ),
        };
        PositionedPort {
            name: p.name.clone(),
            direction: p.direction,
            is_conjugated: p.is_conjugated,
            type_label: p.type_label.clone(),
            side,
            offset: clamped,
            anchor,
            normal,
        }
    };

    let mut out = Vec::new();
    for (p, placement) in &fixed_ports {
        out.push(make_port(p, placement.side, placement.offset));
    }
    for (side, list) in [
        (PortSide::Top, auto_top),
        (PortSide::Right, auto_right),
        (PortSide::Bottom, auto_bottom),
        (PortSide::Left, auto_left),
    ] {
        let count = list.len();
        for (i, p) in list.iter().enumerate() {
            let t = (i + 1) as f32 / (count + 1) as f32;
            out.push(make_port(p, side, t));
        }
    }
    out
}

fn rect_encloses(outer: Rect2, inner: Rect2) -> bool {
    outer.x <= inner.x + 0.5
        && outer.y <= inner.y + 0.5
        && outer.x + outer.w >= inner.x + inner.w - 0.5
        && outer.y + outer.h >= inner.y + inner.h - 0.5
}

fn resolve_edge_anchors(
    src_node: &PositionedNode,
    src_port: Option<&str>,
    dst_node: &PositionedNode,
    dst_port: Option<&str>,
    direction: LayoutDirection,
) -> ((Point2, Point2), (Point2, Point2)) {
    let custom_src = src_port
        .and_then(|name| src_node.ports.iter().find(|p| p.name == name))
        .map(|p| (p.anchor, p.normal));
    let custom_dst = dst_port
        .and_then(|name| dst_node.ports.iter().find(|p| p.name == name))
        .map(|p| (p.anchor, p.normal));

    let src_cy = src_node.rect.y + src_node.rect.h * 0.5;
    let dst_cy = dst_node.rect.y + dst_node.rect.h * 0.5;
    let src_cx = src_node.rect.x + src_node.rect.w * 0.5;
    let dst_cx = dst_node.rect.x + dst_node.rect.w * 0.5;

    let (default_src, default_dst) = match direction {
        LayoutDirection::LeftToRight => {
            if src_cx > dst_cx + 4.0 {
                // Loop back-edge in LR mode: exit bottom, enter bottom.
                (
                    (
                        Point2 {
                            x: src_cx,
                            y: src_node.rect.y + src_node.rect.h,
                        },
                        Point2 { x: 0.0, y: 1.0 },
                    ),
                    (
                        Point2 {
                            x: dst_cx,
                            y: dst_node.rect.y + dst_node.rect.h,
                        },
                        Point2 { x: 0.0, y: 1.0 },
                    ),
                )
            } else {
                (
                    (
                        Point2 {
                            x: src_node.rect.x + src_node.rect.w,
                            y: src_cy,
                        },
                        Point2 { x: 1.0, y: 0.0 },
                    ),
                    (
                        Point2 {
                            x: dst_node.rect.x,
                            y: dst_cy,
                        },
                        Point2 { x: -1.0, y: 0.0 },
                    ),
                )
            }
        }
        LayoutDirection::TopToBottom => {
            if src_cy >= dst_cy - 4.0 {
                // Loop back-edge in TB mode (e.g. decide -> merge): route cleanly along the right side.
                (
                    (
                        Point2 {
                            x: src_node.rect.x + src_node.rect.w,
                            y: src_cy,
                        },
                        Point2 { x: 1.0, y: 0.0 },
                    ),
                    (
                        Point2 {
                            x: dst_node.rect.x + dst_node.rect.w,
                            y: dst_cy,
                        },
                        Point2 { x: 1.0, y: 0.0 },
                    ),
                )
            } else {
                (
                    (
                        Point2 {
                            x: src_cx,
                            y: src_node.rect.y + src_node.rect.h,
                        },
                        Point2 { x: 0.0, y: 1.0 },
                    ),
                    (
                        Point2 {
                            x: dst_cx,
                            y: dst_node.rect.y,
                        },
                        Point2 { x: 0.0, y: -1.0 },
                    ),
                )
            }
        }
    };

    (
        custom_src.unwrap_or(default_src),
        custom_dst.unwrap_or(default_dst),
    )
}

fn segment_intersects_rect(a: Point2, b: Point2, rect: Rect2, pad: f32) -> bool {
    let seg_min_x = a.x.min(b.x);
    let seg_max_x = a.x.max(b.x);
    let seg_min_y = a.y.min(b.y);
    let seg_max_y = a.y.max(b.y);
    let rx0 = rect.x - pad;
    let ry0 = rect.y - pad;
    let rx1 = rect.x + rect.w + pad;
    let ry1 = rect.y + rect.h + pad;
    seg_max_x >= rx0 && seg_min_x <= rx1 && seg_max_y >= ry0 && seg_min_y <= ry1
}

/// Orthogonal A* over visibility-grid channels built from obstacle margins + port coordinates.
#[allow(clippy::too_many_arguments)]
fn route_orthogonal_astar(
    start: Point2,
    start_dir: Point2,
    end: Point2,
    end_dir: Point2,
    obstacles: &[(String, Rect2)],
    excluded_keys: &BTreeSet<String>,
    global_bounds: Rect2,
    lane_offset: f32,
) -> Vec<Point2> {
    let pick_stub = |anchor: Point2, dir: Point2| -> Point2 {
        for stub in [18.0, 12.0, 6.0] {
            let cand = Point2 {
                x: anchor.x + dir.x * stub,
                y: anchor.y + dir.y * stub,
            };
            let inside = obstacles
                .iter()
                .filter(|(k, _)| !excluded_keys.contains(k))
                .any(|(_, r)| r.contains_padded(cand, 2.0));
            if !inside {
                return cand;
            }
        }
        Point2 {
            x: anchor.x + dir.x * 16.0,
            y: anchor.y + dir.y * 16.0,
        }
    };

    let p_start = pick_stub(start, start_dir);
    let p_end = pick_stub(end, end_dir);

    let mut xs = vec![
        start.x,
        p_start.x,
        p_end.x,
        end.x,
        (p_start.x + p_end.x) * 0.5 + lane_offset,
    ];
    let mut ys = vec![
        start.y,
        p_start.y,
        p_end.y,
        end.y,
        (p_start.y + p_end.y) * 0.5 + lane_offset,
    ];

    if global_bounds.w > 0.0 && global_bounds.h > 0.0 {
        xs.push(global_bounds.x - 36.0);
        xs.push(global_bounds.x + global_bounds.w + 36.0);
        ys.push(global_bounds.y - 36.0);
        ys.push(global_bounds.y + global_bounds.h + 36.0);
    }

    let margin = 16.0;
    for (key, rect) in obstacles {
        if excluded_keys.contains(key) {
            continue;
        }
        xs.push(rect.x - margin);
        xs.push(rect.x + rect.w + margin);
        ys.push(rect.y - margin);
        ys.push(rect.y + rect.h + margin);
    }

    xs.sort_by(|a, b| a.total_cmp(b));
    xs.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    ys.sort_by(|a, b| a.total_cmp(b));
    ys.dedup_by(|a, b| (*a - *b).abs() < 0.5);

    let start_idx = nearest_grid_idx(&xs, &ys, p_start);
    let end_idx = nearest_grid_idx(&xs, &ys, p_end);

    for pad in [5.0, 1.5, 0.0] {
        if let Some(grid_path) =
            astar_grid(&xs, &ys, start_idx, end_idx, obstacles, excluded_keys, pad)
        {
            let mut pts = Vec::with_capacity(grid_path.len() + 2);
            pts.push(start);
            for (ix, iy) in grid_path {
                pts.push(Point2 {
                    x: xs[ix],
                    y: ys[iy],
                });
            }
            pts.push(end);
            return simplify_orthogonal(pts);
        }
    }

    // Fallback clean 4-point Manhattan dogleg if obstacles overlap ports tightly.
    let mid_x = (p_start.x + p_end.x) * 0.5;
    simplify_orthogonal(vec![
        start,
        p_start,
        Point2 {
            x: mid_x,
            y: p_start.y,
        },
        Point2 {
            x: mid_x,
            y: p_end.y,
        },
        p_end,
        end,
    ])
}

fn nearest_grid_idx(xs: &[f32], ys: &[f32], target: Point2) -> (usize, usize) {
    let ix = xs
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - target.x).abs().total_cmp(&(**b - target.x).abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let iy = ys
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - target.y).abs().total_cmp(&(**b - target.y).abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);
    (ix, iy)
}

#[derive(Copy, Clone, PartialEq)]
struct GridState {
    cost: f32,
    pos: (usize, usize),
}

impl Eq for GridState {}

impl Ord for GridState {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost).then_with(|| self.pos.cmp(&other.pos))
    }
}

impl PartialOrd for GridState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn astar_grid(
    xs: &[f32],
    ys: &[f32],
    start: (usize, usize),
    goal: (usize, usize),
    obstacles: &[(String, Rect2)],
    excluded_keys: &BTreeSet<String>,
    pad: f32,
) -> Option<Vec<(usize, usize)>> {
    let mut dist: BTreeMap<(usize, usize), f32> = BTreeMap::new();
    let mut came_from: BTreeMap<(usize, usize), (usize, usize)> = BTreeMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(start, 0.0);
    heap.push(GridState {
        cost: 0.0,
        pos: start,
    });

    while let Some(GridState { pos, .. }) = heap.pop() {
        if pos == goal {
            let mut path = vec![goal];
            let mut cur = goal;
            while let Some(&prev) = came_from.get(&cur) {
                path.push(prev);
                cur = prev;
            }
            path.reverse();
            return Some(path);
        }

        let cur_g = *dist.get(&pos).unwrap_or(&f32::MAX);
        let (ix, iy) = pos;
        let mut neighbors = Vec::with_capacity(4);
        if ix > 0 {
            neighbors.push((ix - 1, iy));
        }
        if ix + 1 < xs.len() {
            neighbors.push((ix + 1, iy));
        }
        if iy > 0 {
            neighbors.push((ix, iy - 1));
        }
        if iy + 1 < ys.len() {
            neighbors.push((ix, iy + 1));
        }

        for next in neighbors {
            let a = Point2 { x: xs[ix], y: ys[iy] };
            let b = Point2 {
                x: xs[next.0],
                y: ys[next.1],
            };
            let blocked = obstacles
                .iter()
                .filter(|(k, _)| !excluded_keys.contains(k))
                .any(|(_, r)| segment_intersects_rect(a, b, *r, pad));
            if blocked {
                continue;
            }

            let step = (a.x - b.x).abs() + (a.y - b.y).abs();
            let turn_penalty = if let Some(&prev) = came_from.get(&pos) {
                let same_axis = (prev.0 == pos.0 && pos.0 == next.0)
                    || (prev.1 == pos.1 && pos.1 == next.1);
                if same_axis { 0.0 } else { 12.0 }
            } else {
                0.0
            };
            let next_g = cur_g + step + turn_penalty;
            if next_g < *dist.get(&next).unwrap_or(&f32::MAX) {
                dist.insert(next, next_g);
                came_from.insert(next, pos);
                let h = (xs[next.0] - xs[goal.0]).abs() + (ys[next.1] - ys[goal.1]).abs();
                heap.push(GridState {
                    cost: next_g + h,
                    pos: next,
                });
            }
        }
    }
    None
}

fn simplify_orthogonal(points: Vec<Point2>) -> Vec<Point2> {
    if points.len() <= 2 {
        return points;
    }
    let mut out: Vec<Point2> = Vec::with_capacity(points.len());
    for p in points {
        if let Some(last) = out.last() {
            if (last.x - p.x).abs() < 0.01 && (last.y - p.y).abs() < 0.01 {
                continue;
            }
        }
        out.push(p);
        while out.len() >= 3 {
            let n = out.len();
            let a = out[n - 3];
            let b = out[n - 2];
            let c = out[n - 1];
            let collinear_x = (a.x - b.x).abs() < 0.01 && (b.x - c.x).abs() < 0.01;
            let collinear_y = (a.y - b.y).abs() < 0.01 && (b.y - c.y).abs() < 0.01;
            if collinear_x || collinear_y {
                out.remove(n - 2);
            } else {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use syssur_model::ElementId;

    fn sample_input() -> LayoutInput {
        let p1 = InstancePath::root(ElementId::new("Power::battery"));
        let p2 = InstancePath::root(ElementId::new("Power::motor"));
        let p3 = InstancePath::root(ElementId::new("Power::controller"));
        LayoutInput {
            direction: LayoutDirection::LeftToRight,
            nodes: vec![
                LayoutNodeSpec {
                    path: p1.clone(),
                    parent: None,
                    label: "battery".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "powerOut".into(),
                        direction: PortDirection::Out,
                        is_conjugated: false,
                        type_label: Some("PowerPort".into()),
                        span: Span::new("power.sysml", 10, 20),
                    }],
                    span: Span::new("power.sysml", 5, 30),
                },
                LayoutNodeSpec {
                    path: p2.clone(),
                    parent: None,
                    label: "motor".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "powerIn".into(),
                        direction: PortDirection::Out,
                        is_conjugated: true,
                        type_label: Some("~PowerPort".into()),
                        span: Span::new("power.sysml", 45, 55),
                    }],
                    span: Span::new("power.sysml", 40, 60),
                },
                LayoutNodeSpec {
                    path: p3.clone(),
                    parent: None,
                    label: "controller".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 80.0,
                    ports: vec![],
                    span: Span::new("power.sysml", 70, 90),
                },
            ],
            edges: vec![LayoutEdgeSpec {
                id: "powerBus".into(),
                source_node: p1,
                source_port: Some("powerOut".into()),
                target_node: p2,
                target_port: Some("powerIn".into()),
                kind: RelationshipKind::Interface,
                label: Some("PowerInterface".into()),
                guard: None,
                span: Span::new("power.sysml", 100, 120),
            }],
        }
    }

    #[test]
    fn layout_is_bit_for_bit_identical_regardless_of_input_order() {
        let forward = sample_input();
        let mut reversed = forward.clone();
        reversed.nodes.reverse();
        reversed.edges.reverse();

        let out_a = compute_deterministic_layout(&forward);
        let out_b = compute_deterministic_layout(&reversed);
        assert_eq!(out_a, out_b);

        for edge in &out_a.edges {
            assert!(edge.waypoints.len() >= 2);
            for w in edge.waypoints.windows(2) {
                let dx = (w[0].x - w[1].x).abs();
                let dy = (w[0].y - w[1].y).abs();
                assert!(
                    dx < 0.1 || dy < 0.1,
                    "non-orthogonal segment: {:?} -> {:?}",
                    w[0],
                    w[1]
                );
            }
        }
    }

    #[test]
    fn top_to_bottom_action_flow_with_loop_back_edge_is_orthogonal() {
        let root = InstancePath::root(ElementId::new("Behavior::Story"));
        let start = root.push(ElementId::new("start"));
        let merge = root.push(ElementId::new("mergeChime"));
        let act = root.push(ElementId::new("BeginChime"));
        let decide = root.push(ElementId::new("decideChime"));
        let input = LayoutInput {
            direction: LayoutDirection::TopToBottom,
            nodes: vec![
                LayoutNodeSpec {
                    path: root.clone(),
                    parent: None,
                    label: "Story".into(),
                    shape: NodeShape::ActionBox,
                    width: 240.0,
                    height: 180.0,
                    ports: vec![],
                    span: Span::new("b.sysml", 1, 10),
                },
                LayoutNodeSpec {
                    path: start.clone(),
                    parent: Some(root.clone()),
                    label: "start".into(),
                    shape: NodeShape::StartCircle,
                    width: 28.0,
                    height: 28.0,
                    ports: vec![],
                    span: Span::new("b.sysml", 12, 20),
                },
                LayoutNodeSpec {
                    path: merge.clone(),
                    parent: Some(root.clone()),
                    label: "mergeChime".into(),
                    shape: NodeShape::MergeDiamond,
                    width: 36.0,
                    height: 36.0,
                    ports: vec![],
                    span: Span::new("b.sysml", 22, 30),
                },
                LayoutNodeSpec {
                    path: act.clone(),
                    parent: Some(root.clone()),
                    label: "BeginChime".into(),
                    shape: NodeShape::ActionBox,
                    width: 160.0,
                    height: 48.0,
                    ports: vec![],
                    span: Span::new("b.sysml", 32, 40),
                },
                LayoutNodeSpec {
                    path: decide.clone(),
                    parent: Some(root),
                    label: "decideChime".into(),
                    shape: NodeShape::DecideDiamond,
                    width: 36.0,
                    height: 36.0,
                    ports: vec![],
                    span: Span::new("b.sysml", 42, 50),
                },
            ],
            edges: vec![
                LayoutEdgeSpec {
                    id: "e1".into(),
                    source_node: start,
                    source_port: None,
                    target_node: merge.clone(),
                    target_port: None,
                    kind: RelationshipKind::Succession,
                    label: None,
                    guard: None,
                    span: Span::new("b.sysml", 60, 70),
                },
                LayoutEdgeSpec {
                    id: "e2".into(),
                    source_node: merge.clone(),
                    source_port: None,
                    target_node: act.clone(),
                    target_port: None,
                    kind: RelationshipKind::Succession,
                    label: None,
                    guard: None,
                    span: Span::new("b.sysml", 72, 80),
                },
                LayoutEdgeSpec {
                    id: "e3".into(),
                    source_node: act,
                    source_port: None,
                    target_node: decide.clone(),
                    target_port: None,
                    kind: RelationshipKind::Succession,
                    label: None,
                    guard: None,
                    span: Span::new("b.sysml", 82, 90),
                },
                LayoutEdgeSpec {
                    id: "e4_loop".into(),
                    source_node: decide,
                    source_port: None,
                    target_node: merge,
                    target_port: None,
                    kind: RelationshipKind::Succession,
                    label: None,
                    guard: Some("timeElapsed == false".into()),
                    span: Span::new("b.sysml", 92, 110),
                },
            ],
        };

        let out = compute_deterministic_layout(&input);
        assert_eq!(out.edges.len(), 4);
        assert_eq!(out.edges[3].guard.as_deref(), Some("timeElapsed == false"));
        for edge in &out.edges {
            for w in edge.waypoints.windows(2) {
                let dx = (w[0].x - w[1].x).abs();
                let dy = (w[0].y - w[1].y).abs();
                assert!(dx < 0.1 || dy < 0.1);
            }
        }
    }

    #[test]
    fn node_offsets_move_node_and_reroute_orthogonally_then_reset() {
        let input = sample_input();
        let baseline = compute_deterministic_layout(&input);
        let key = "Power::battery".to_string();
        let base_rect = baseline.nodes.get(&key).unwrap().rect;

        let mut offsets = BTreeMap::new();
        offsets.insert(key.clone(), Point2 { x: 80.0, y: 40.0 });
        let moved = compute_layout_with_overrides(&input, &offsets);
        let moved_rect = moved.nodes.get(&key).unwrap().rect;

        assert!((moved_rect.x - (base_rect.x + 80.0)).abs() < 0.01);
        assert!((moved_rect.y - (base_rect.y + 40.0)).abs() < 0.01);
        for edge in &moved.edges {
            assert!(edge.waypoints.len() >= 2);
            for w in edge.waypoints.windows(2) {
                let dx = (w[0].x - w[1].x).abs();
                let dy = (w[0].y - w[1].y).abs();
                assert!(dx < 0.1 || dy < 0.1);
            }
        }

        let reset = compute_layout_with_overrides(&input, &BTreeMap::new());
        assert_eq!(baseline, reset);
    }

    #[test]
    fn routed_edge_never_crosses_source_target_or_intermediate_blocks() {
        let p_src = InstancePath::root(ElementId::new("Pkg::src"));
        let p_mid = InstancePath::root(ElementId::new("Pkg::blocker"));
        let p_dst = InstancePath::root(ElementId::new("Pkg::dst"));
        let input = LayoutInput {
            direction: LayoutDirection::LeftToRight,
            nodes: vec![
                LayoutNodeSpec {
                    path: p_src.clone(),
                    parent: None,
                    label: "src".into(),
                    shape: NodeShape::PartBlock,
                    width: 150.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "pOut".into(),
                        direction: PortDirection::Out,
                        is_conjugated: false,
                        type_label: None,
                        span: Span::new("m.sysml", 1, 5),
                    }],
                    span: Span::new("m.sysml", 0, 10),
                },
                LayoutNodeSpec {
                    path: p_mid.clone(),
                    parent: None,
                    label: "blocker".into(),
                    shape: NodeShape::PartBlock,
                    width: 150.0,
                    height: 80.0,
                    ports: vec![],
                    span: Span::new("m.sysml", 12, 20),
                },
                LayoutNodeSpec {
                    path: p_dst.clone(),
                    parent: None,
                    label: "dst".into(),
                    shape: NodeShape::PartBlock,
                    width: 150.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "pIn".into(),
                        direction: PortDirection::In,
                        is_conjugated: false,
                        type_label: None,
                        span: Span::new("m.sysml", 22, 28),
                    }],
                    span: Span::new("m.sysml", 21, 30),
                },
            ],
            edges: vec![LayoutEdgeSpec {
                id: "link".into(),
                source_node: p_src.clone(),
                source_port: Some("pOut".into()),
                target_node: p_dst.clone(),
                target_port: Some("pIn".into()),
                kind: RelationshipKind::Connection,
                label: None,
                guard: None,
                span: Span::new("m.sysml", 40, 50),
            }],
        };

        let mut base = compute_deterministic_layout(&input);
        let src_r = base.nodes[&p_src.canonical_key()].rect;
        let dst_r = base.nodes[&p_dst.canonical_key()].rect;
        let mid_r = base.nodes[&p_mid.canonical_key()].rect;
        let mut node_offsets = BTreeMap::new();
        node_offsets.insert(
            p_mid.canonical_key(),
            Point2 {
                x: (src_r.x + dst_r.x) * 0.5 - mid_r.x,
                y: (src_r.y + dst_r.y) * 0.5 - mid_r.y,
            },
        );
        let mut port_overrides = BTreeMap::new();
        port_overrides.insert(
            (p_src.canonical_key(), "pOut".into()),
            PortPlacement {
                side: PortSide::Left,
                offset: 0.5,
            },
        );
        port_overrides.insert(
            (p_dst.canonical_key(), "pIn".into()),
            PortPlacement {
                side: PortSide::Right,
                offset: 0.5,
            },
        );

        base = compute_layout_with_port_overrides(&input, &node_offsets, &port_overrides);
        let wps = &base.edges[0].waypoints;
        assert!(wps.len() >= 4);
        for seg in wps[1..wps.len() - 1].windows(2) {
            for node in base.nodes.values() {
                assert!(
                    !segment_intersects_rect(seg[0], seg[1], node.rect, -0.5),
                    "edge segment {:?} -> {:?} crossed block '{}' {:?}",
                    seg[0],
                    seg[1],
                    node.label,
                    node.rect
                );
            }
        }
    }

    #[test]
    fn port_placement_faces_connected_neighbor_and_honors_user_override() {
        let p_a = InstancePath::root(ElementId::new("Pkg::a"));
        let p_b = InstancePath::root(ElementId::new("Pkg::b"));
        let input = LayoutInput {
            direction: LayoutDirection::LeftToRight,
            nodes: vec![
                LayoutNodeSpec {
                    path: p_a.clone(),
                    parent: None,
                    label: "a".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "ioPort".into(),
                        direction: PortDirection::InOut,
                        is_conjugated: false,
                        type_label: None,
                        span: Span::new("m.sysml", 5, 10),
                    }],
                    span: Span::new("m.sysml", 1, 12),
                },
                LayoutNodeSpec {
                    path: p_b.clone(),
                    parent: None,
                    label: "b".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 80.0,
                    ports: vec![LayoutPortSpec {
                        name: "ioIn".into(),
                        direction: PortDirection::InOut,
                        is_conjugated: false,
                        type_label: None,
                        span: Span::new("m.sysml", 20, 25),
                    }],
                    span: Span::new("m.sysml", 15, 28),
                },
            ],
            edges: vec![LayoutEdgeSpec {
                id: "ab".into(),
                source_node: p_a.clone(),
                source_port: Some("ioPort".into()),
                target_node: p_b.clone(),
                target_port: Some("ioIn".into()),
                kind: RelationshipKind::Connection,
                label: None,
                guard: None,
                span: Span::new("m.sysml", 30, 40),
            }],
        };

        let out = compute_deterministic_layout(&input);
        let port_a = &out.nodes[&p_a.canonical_key()].ports[0];
        let port_b = &out.nodes[&p_b.canonical_key()].ports[0];
        assert_eq!(port_a.side, PortSide::Right);
        assert_eq!(port_b.side, PortSide::Left);

        let mut ovr = BTreeMap::new();
        ovr.insert(
            (p_a.canonical_key(), "ioPort".into()),
            PortPlacement {
                side: PortSide::Top,
                offset: 0.3,
            },
        );
        let out_ovr = compute_layout_with_port_overrides(&input, &BTreeMap::new(), &ovr);
        let port_a_ovr = &out_ovr.nodes[&p_a.canonical_key()].ports[0];
        assert_eq!(port_a_ovr.side, PortSide::Top);
        assert!((port_a_ovr.offset - 0.3).abs() < 0.01);
        let node_a_rect = out_ovr.nodes[&p_a.canonical_key()].rect;
        assert!((port_a_ovr.anchor.x - (node_a_rect.x + node_a_rect.w * 0.3)).abs() < 0.5);
        assert!((port_a_ovr.anchor.y - node_a_rect.y).abs() < 0.5);
    }

    #[test]
    fn three_level_compound_containers_enclose_descendants_bottom_up() {
        let root = InstancePath::root(ElementId::new("RoverSystem::InspectionRover"));
        let sub = root.push(ElementId::new("RoverSystem::InspectionRover::energy"));
        let leaf_a = sub.push(ElementId::new("RoverSystem::InspectionRover::energy::battery"));
        let leaf_b = sub.push(ElementId::new("RoverSystem::InspectionRover::energy::distribution"));

        let input = LayoutInput {
            direction: LayoutDirection::LeftToRight,
            nodes: vec![
                LayoutNodeSpec {
                    path: root.clone(),
                    parent: None,
                    label: "InspectionRover".into(),
                    shape: NodeShape::PartBlock,
                    width: 160.0,
                    height: 80.0,
                    ports: vec![],
                    span: Span::new("s.sysml", 1, 20),
                },
                LayoutNodeSpec {
                    path: sub.clone(),
                    parent: Some(root.clone()),
                    label: "energy".into(),
                    shape: NodeShape::PartBlock,
                    width: 140.0,
                    height: 60.0,
                    ports: vec![],
                    span: Span::new("s.sysml", 25, 40),
                },
                LayoutNodeSpec {
                    path: leaf_a.clone(),
                    parent: Some(sub.clone()),
                    label: "battery".into(),
                    shape: NodeShape::PartBlock,
                    width: 160.0,
                    height: 90.0,
                    ports: vec![LayoutPortSpec {
                        name: "supply".into(),
                        direction: PortDirection::Out,
                        is_conjugated: false,
                        type_label: Some("PowerPort".into()),
                        span: Span::new("s.sysml", 45, 50),
                    }],
                    span: Span::new("s.sysml", 42, 60),
                },
                LayoutNodeSpec {
                    path: leaf_b.clone(),
                    parent: Some(sub.clone()),
                    label: "distribution".into(),
                    shape: NodeShape::PartBlock,
                    width: 160.0,
                    height: 90.0,
                    ports: vec![LayoutPortSpec {
                        name: "input".into(),
                        direction: PortDirection::In,
                        is_conjugated: false,
                        type_label: Some("PowerPort".into()),
                        span: Span::new("s.sysml", 65, 70),
                    }],
                    span: Span::new("s.sysml", 62, 80),
                },
            ],
            edges: vec![LayoutEdgeSpec {
                id: "batterySupply#0".into(),
                source_node: leaf_a.clone(),
                source_port: Some("supply".into()),
                target_node: leaf_b.clone(),
                target_port: Some("input".into()),
                kind: RelationshipKind::Connection,
                label: Some("batterySupply".into()),
                guard: None,
                span: Span::new("s.sysml", 82, 95),
            }],
        };

        let out = compute_deterministic_layout(&input);
        let r_root = out.nodes[&root.canonical_key()].rect;
        let r_sub = out.nodes[&sub.canonical_key()].rect;
        let r_a = out.nodes[&leaf_a.canonical_key()].rect;
        let r_b = out.nodes[&leaf_b.canonical_key()].rect;

        assert!(
            r_sub.x <= r_a.x - 20.0 && r_sub.y <= r_a.y - 40.0,
            "sub container must enclose leaf_a with header clearance: sub={r_sub:?}, a={r_a:?}"
        );
        assert!(
            r_sub.x + r_sub.w >= r_b.x + r_b.w + 20.0
                && r_sub.y + r_sub.h >= r_b.y + r_b.h + 18.0,
            "sub container must enclose leaf_b: sub={r_sub:?}, b={r_b:?}"
        );
        assert!(
            r_root.x <= r_sub.x - 20.0
                && r_root.y <= r_sub.y - 40.0
                && r_root.x + r_root.w >= r_sub.x + r_sub.w + 20.0
                && r_root.y + r_root.h >= r_sub.y + r_sub.h + 18.0,
            "root container must enclose sub container: root={r_root:?}, sub={r_sub:?}"
        );
        assert_eq!(out.edges.len(), 1);
        assert!(out.edges[0].waypoints.len() >= 2);
    }
}
