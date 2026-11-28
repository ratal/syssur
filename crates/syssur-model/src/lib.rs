//! Core Syssur domain model separating the SysML v2 Definition Graph
//! from the dynamic Contextual Instance Tree (`InstancePath`), view filters,
//! and cross-model traceability graph.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// SysML v2 element identifier using its canonical Qualified Name
/// (e.g. `RoverSystem::Rover::powertrain`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ElementId(pub String);

impl ElementId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the short local name after the final `::`.
    pub fn short_name(&self) -> &str {
        self.0.rsplit("::").next().unwrap_or(&self.0)
    }

    /// Joins a child segment onto this qualified name.
    pub fn child(&self, segment: &str) -> Self {
        if self.0.is_empty() {
            Self(segment.to_string())
        } else {
            Self(format!("{}::{}", self.0, segment))
        }
    }
}

impl fmt::Display for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Contextual occurrence path identifying a specific instance in an expanded
/// system hierarchy (e.g. `[Rover, powertrain, motor]`), so multiple usages of
/// the same definition remain independent.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InstancePath(pub Vec<ElementId>);

impl InstancePath {
    pub fn root(id: ElementId) -> Self {
        Self(vec![id])
    }

    pub fn push(&self, segment: ElementId) -> Self {
        let mut next = self.0.clone();
        next.push(segment);
        Self(next)
    }

    pub fn leaf(&self) -> Option<&ElementId> {
        self.0.last()
    }

    pub fn parent(&self) -> Option<Self> {
        if self.0.len() <= 1 {
            None
        } else {
            Some(Self(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    pub fn canonical_key(&self) -> String {
        self.0
            .iter()
            .map(ElementId::as_str)
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl fmt::Display for InstancePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let display = self
            .0
            .iter()
            .map(ElementId::short_name)
            .collect::<Vec<_>>()
            .join(".");
        f.write_str(&display)
    }
}

/// Source location used for diagnostics and canonical AST-span layout sorting.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(file: impl Into<String>, start: usize, end: usize) -> Self {
        Self {
            file: file.into(),
            start,
            end,
        }
    }
}

/// Tracks whether a feature is locally owned, inherited, subsetted (`:>`), or redefined (`:>>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureOrigin {
    Owned,
    Inherited(ElementId),
    Subsetted(ElementId),
    Redefined(ElementId),
}

/// Port direction before/after SysML v2 conjugation (`~`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortDirection {
    In,
    Out,
    InOut,
}

impl PortDirection {
    /// Mirrors `In <-> Out` when the port usage is conjugated (`~PortDef`).
    pub fn effective_direction(self, is_conjugated: bool) -> Self {
        if !is_conjugated {
            return self;
        }
        match self {
            Self::In => Self::Out,
            Self::Out => Self::In,
            Self::InOut => Self::InOut,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipKind {
    Composition,
    Specialization,
    Subsetting,
    Redefinition,
    Connection,
    Interface,
    Flow,
    Succession,
    Transition,
    ReferenceSubsetting,
    Satisfy,
    Verify,
    Allocate,
    Dependency,
    Binding,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewKind {
    #[default]
    General,
    Interconnection,
    ActionFlow,
    StateTransition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionNodeKind {
    Action,
    Start,
    Done,
    Fork,
    Join,
    Decide,
    Merge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionNode {
    pub id: ElementId,
    pub name: String,
    pub kind: ActionNodeKind,
    #[serde(default)]
    pub is_def: bool,
    #[serde(default)]
    pub type_def: Option<ElementId>,
    #[serde(default)]
    pub params: Vec<PortUsage>,
    pub doc: Option<String>,
    pub children: Vec<ActionNode>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuccessionEdge {
    pub id: ElementId,
    pub owner_scope: ElementId,
    pub source: String,
    pub target: String,
    pub guard: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateNode {
    pub id: ElementId,
    pub name: String,
    pub is_def: bool,
    pub doc: Option<String>,
    pub children: Vec<StateNode>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionEdge {
    pub id: ElementId,
    pub owner_scope: ElementId,
    #[serde(default)]
    pub name: Option<String>,
    pub source: String,
    pub target: String,
    pub trigger: Option<String>,
    pub guard: Option<String>,
    pub effect: Option<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SysmlElementKind {
    Attribute,
    Enum,
    Item,
    Occurrence,
    Connection,
    Interface,
    Allocation,
    Flow,
    Constraint,
    Concern,
    Calc,
    Case,
    UseCase,
    AnalysisCase,
    VerificationCase,
    Viewpoint,
    Rendering,
    Metadata,
    Comment,
    Alias,
    Filter,
    TextualRep,
    PerformAction,
    AcceptAction,
    SendAction,
    AssignAction,
    TerminateAction,
    IfAction,
    LoopAction,
    ExhibitState,
    IncludeUseCase,
    AssertConstraint,
    Subject,
    Actor,
    Stakeholder,
    EventOccurrence,
    Timeslice,
    Snapshot,
    RefUsage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementModifiers {
    pub visibility: Option<String>,
    pub direction: Option<PortDirection>,
    pub is_abstract: bool,
    pub is_variation: bool,
    pub is_variant: bool,
    pub is_individual: bool,
    pub is_ref: bool,
    pub is_derived: bool,
    pub is_constant: bool,
    pub is_parallel: bool,
    pub value_expr: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SysmlElement {
    pub id: ElementId,
    pub name: String,
    pub kind: SysmlElementKind,
    pub is_def: bool,
    pub short_id: Option<String>,
    pub type_ref: Option<ElementId>,
    pub specializes: Vec<ElementId>,
    pub doc: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageNode {
    pub id: ElementId,
    pub name: String,
    pub imports: Vec<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDef {
    pub id: ElementId,
    pub name: String,
    pub direction: PortDirection,
    pub item_type: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortUsage {
    pub id: ElementId,
    pub name: String,
    pub port_def: Option<ElementId>,
    pub is_conjugated: bool,
    pub direction: PortDirection,
    pub origin: FeatureOrigin,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeUsage {
    pub id: ElementId,
    pub name: String,
    pub type_name: Option<String>,
    pub value_expr: Option<String>,
    pub origin: FeatureOrigin,
    pub span: Span,
}

impl AttributeUsage {
    /// Concise SysML v2 compartment label (`name [: Type] [= val]`).
    pub fn display_label(&self) -> String {
        let mut s = self.name.clone();
        if let Some(ref ty) = self.type_name {
            if !ty.is_empty() {
                if ty.starts_with(":>") || ty.starts_with(":>>") {
                    s.push(' ');
                    s.push_str(ty);
                } else {
                    s.push_str(" : ");
                    s.push_str(ty);
                }
            }
        }
        if let Some(ref val) = self.value_expr {
            if !val.is_empty() {
                s.push_str(" = ");
                s.push_str(val);
            }
        }
        s
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartUsage {
    pub id: ElementId,
    pub name: String,
    pub type_def: Option<ElementId>,
    pub multiplicity: Option<u32>,
    pub origin: FeatureOrigin,
    pub ports: Vec<PortUsage>,
    pub nested_parts: Vec<PartUsage>,
    #[serde(default)]
    pub attributes: Vec<AttributeUsage>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartDef {
    pub id: ElementId,
    pub name: String,
    pub specializes: Vec<ElementId>,
    pub ports: Vec<PortUsage>,
    pub parts: Vec<PartUsage>,
    pub attributes: Vec<String>,
    #[serde(default)]
    pub attribute_usages: Vec<AttributeUsage>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionEdge {
    pub id: ElementId,
    pub name: Option<String>,
    pub kind: RelationshipKind,
    pub interface_def: Option<ElementId>,
    pub owner_scope: ElementId,
    pub source_path: String,
    pub target_path: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementSubject {
    pub name: String,
    #[serde(default)]
    pub type_def: Option<ElementId>,
    #[serde(default)]
    pub value_expr: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementNode {
    pub id: ElementId,
    pub short_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub type_def: Option<ElementId>,
    #[serde(default)]
    pub subject: Option<RequirementSubject>,
    #[serde(default)]
    pub attributes: Vec<AttributeUsage>,
    #[serde(default)]
    pub children: Vec<ElementId>,
    pub doc: Option<String>,
    pub is_usage: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceLink {
    pub kind: RelationshipKind,
    pub requirement: ElementId,
    pub target: String,
    #[serde(default)]
    pub is_asserted: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDef {
    pub id: ElementId,
    pub name: String,
    pub kind: ViewKind,
    #[serde(default)]
    pub base_views: Vec<ElementId>,
    pub is_usage: bool,
    pub expose_patterns: Vec<String>,
    pub include_relationships: Vec<RelationshipKind>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticItem {
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationStatus {
    Satisfied,
    Violated,
    Undecided(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstraintVerdictItem {
    pub name: String,
    pub context: Option<String>,
    pub status: VerificationStatus,
    pub span: Span,
}

/// Extracted SysML v2 semantic graph of definitions, usages, connections,
/// requirements, and views across a workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinitionGraph {
    pub packages: BTreeMap<ElementId, PackageNode>,
    pub part_defs: BTreeMap<ElementId, PartDef>,
    pub port_defs: BTreeMap<ElementId, PortDef>,
    pub top_level_parts: BTreeMap<ElementId, PartUsage>,
    pub connections: Vec<ConnectionEdge>,
    pub actions: BTreeMap<ElementId, ActionNode>,
    pub successions: Vec<SuccessionEdge>,
    pub states: BTreeMap<ElementId, StateNode>,
    pub transitions: Vec<TransitionEdge>,
    pub generic_elements: BTreeMap<ElementId, SysmlElement>,
    pub requirements: BTreeMap<ElementId, RequirementNode>,
    pub trace_links: Vec<TraceLink>,
    pub views: BTreeMap<ElementId, ViewDef>,
    pub element_modifiers: BTreeMap<ElementId, ElementModifiers>,
    pub diagnostics: Vec<DiagnosticItem>,
    pub verdicts: Vec<ConstraintVerdictItem>,
}

impl DefinitionGraph {
    /// Looks up a `PartDef` by exact `ElementId` or unique short name across packages.
    pub fn resolve_part_def(&self, target: &ElementId) -> Option<&PartDef> {
        if let Some(found) = self.part_defs.get(target) {
            return Some(found);
        }
        let short = target.short_name();
        self.part_defs
            .values()
            .find(|def| def.name == short || def.id.as_str().ends_with(&format!("::{short}")))
    }

    /// Looks up a top-level `ActionNode` by exact `ElementId` or unique short name.
    pub fn resolve_action(&self, target: &ElementId) -> Option<&ActionNode> {
        if let Some(found) = self.actions.get(target) {
            return Some(found);
        }
        let short = target.short_name();
        self.actions
            .values()
            .find(|act| act.name == short || act.id.as_str().ends_with(&format!("::{short}")))
    }

    /// Looks up a top-level `StateNode` by exact `ElementId` or unique short name.
    pub fn resolve_state(&self, target: &ElementId) -> Option<&StateNode> {
        if let Some(found) = self.states.get(target) {
            return Some(found);
        }
        let short = target.short_name();
        self.states
            .values()
            .find(|st| st.name == short || st.id.as_str().ends_with(&format!("::{short}")))
    }

    /// Looks up a `RequirementNode` by exact `ElementId` or unique short name.
    pub fn resolve_requirement(&self, target: &ElementId) -> Option<&RequirementNode> {
        if let Some(found) = self.requirements.get(target) {
            return Some(found);
        }
        let short = target.short_name();
        self.requirements
            .values()
            .find(|req| req.name == short || req.id.as_str().ends_with(&format!("::{short}")))
    }

    /// Looks up a `PortDef` by exact `ElementId` or unique short name.
    pub fn resolve_port_def(&self, target: &ElementId) -> Option<&PortDef> {
        if let Some(found) = self.port_defs.get(target) {
            return Some(found);
        }
        let short = target.short_name();
        self.port_defs
            .values()
            .find(|def| def.name == short || def.id.as_str().ends_with(&format!("::{short}")))
    }

    /// Collects effective parts (inherited from base `specializes` + local overrides/redefinitions).
    pub fn effective_parts_for_def(&self, def_id: &ElementId) -> Vec<PartUsage> {
        let mut visited = BTreeSet::new();
        self.collect_parts_recursive(def_id, &mut visited)
    }

    fn collect_parts_recursive(
        &self,
        def_id: &ElementId,
        visited: &mut BTreeSet<ElementId>,
    ) -> Vec<PartUsage> {
        let Some(def) = self.resolve_part_def(def_id) else {
            return Vec::new();
        };
        if !visited.insert(def.id.clone()) {
            return Vec::new();
        }

        let mut by_name: BTreeMap<String, PartUsage> = BTreeMap::new();
        for base in &def.specializes {
            for inherited in self.collect_parts_recursive(base, visited) {
                let mut marked = inherited;
                if matches!(marked.origin, FeatureOrigin::Owned) {
                    marked.origin = FeatureOrigin::Inherited(def.id.clone());
                }
                by_name.insert(marked.name.clone(), marked);
            }
        }

        for local in &def.parts {
            let mut item = local.clone();
            if matches!(item.origin, FeatureOrigin::Redefined(_)) {
                if let Some(prev) = by_name.get(&item.name) {
                    if item.type_def.is_none() {
                        item.type_def = prev.type_def.clone();
                    }
                    if item.nested_parts.is_empty() {
                        item.nested_parts = prev.nested_parts.clone();
                    }
                }
            }
            by_name.insert(item.name.clone(), item);
        }

        let mut result: Vec<PartUsage> = by_name.into_values().collect();
        result.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.name.cmp(&b.name)));
        result
    }

    /// Collects effective attributes (inherited + locally defined/redefined).
    pub fn effective_attributes_for_def(&self, def_id: &ElementId) -> Vec<AttributeUsage> {
        let mut visited = BTreeSet::new();
        self.collect_attributes_recursive(def_id, &mut visited)
    }

    fn collect_attributes_recursive(
        &self,
        def_id: &ElementId,
        visited: &mut BTreeSet<ElementId>,
    ) -> Vec<AttributeUsage> {
        let Some(def) = self.resolve_part_def(def_id) else {
            return Vec::new();
        };
        if !visited.insert(def.id.clone()) {
            return Vec::new();
        }

        let mut by_name: BTreeMap<String, AttributeUsage> = BTreeMap::new();
        for base in &def.specializes {
            for mut inherited in self.collect_attributes_recursive(base, visited) {
                if matches!(inherited.origin, FeatureOrigin::Owned) {
                    inherited.origin = FeatureOrigin::Inherited(def.id.clone());
                }
                by_name.insert(inherited.name.clone(), inherited);
            }
        }
        for local in &def.attribute_usages {
            let mut item = local.clone();
            if matches!(item.origin, FeatureOrigin::Redefined(_)) {
                if let Some(prev) = by_name.get(&item.name) {
                    if item.type_name.is_none() {
                        item.type_name = prev.type_name.clone();
                    }
                    if item.value_expr.is_none() {
                        item.value_expr = prev.value_expr.clone();
                    }
                }
            }
            by_name.insert(item.name.clone(), item);
        }
        let mut result: Vec<AttributeUsage> = by_name.into_values().collect();
        result.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.name.cmp(&b.name)));
        result
    }

    /// Collects effective ports (inherited + locally defined/redefined) with direction resolved.
    pub fn effective_ports_for_def(&self, def_id: &ElementId) -> Vec<PortUsage> {
        let mut visited = BTreeSet::new();
        self.collect_ports_recursive(def_id, &mut visited)
    }

    fn collect_ports_recursive(
        &self,
        def_id: &ElementId,
        visited: &mut BTreeSet<ElementId>,
    ) -> Vec<PortUsage> {
        let Some(def) = self.resolve_part_def(def_id) else {
            return Vec::new();
        };
        if !visited.insert(def.id.clone()) {
            return Vec::new();
        }

        let mut by_name: BTreeMap<String, PortUsage> = BTreeMap::new();
        for base in &def.specializes {
            for mut inherited in self.collect_ports_recursive(base, visited) {
                if matches!(inherited.origin, FeatureOrigin::Owned) {
                    inherited.origin = FeatureOrigin::Inherited(def.id.clone());
                }
                by_name.insert(inherited.name.clone(), inherited);
            }
        }
        for local in &def.ports {
            let mut resolved = local.clone();
            if resolved.direction == PortDirection::InOut {
                if let Some(ref pd_id) = resolved.port_def {
                    if let Some(pd) = self.resolve_port_def(pd_id) {
                        resolved.direction = pd.direction.effective_direction(resolved.is_conjugated);
                    }
                }
            }
            by_name.insert(resolved.name.clone(), resolved);
        }
        let mut result: Vec<PortUsage> = by_name.into_values().collect();
        result.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.name.cmp(&b.name)));
        result
    }

    /// Checks `PortDef` type and effective conjugated direction compatibility between two ports.
    pub fn check_port_compatibility(&self, src: &PortUsage, dst: &PortUsage) -> PortCompatibility {
        let src_type = src.port_def.as_ref().map(|id| id.short_name());
        let dst_type = dst.port_def.as_ref().map(|id| id.short_name());

        let resolve_dir = |p: &PortUsage| -> PortDirection {
            if p.direction != PortDirection::InOut {
                p.direction
            } else {
                p.port_def
                    .as_ref()
                    .and_then(|id| self.resolve_port_def(id))
                    .map(|pd| pd.direction.effective_direction(p.is_conjugated))
                    .unwrap_or_else(|| p.direction.effective_direction(p.is_conjugated))
            }
        };

        let src_dir = resolve_dir(src);
        let dst_dir = resolve_dir(dst);

        check_port_direction_and_type(src_type, src_dir, dst_type, dst_dir)
    }
}

/// Result of checking whether two SysML v2 ports can be wired together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortCompatibility {
    Compatible,
    DirectionMismatch {
        source_dir: PortDirection,
        target_dir: PortDirection,
    },
    TypeMismatch {
        source_type: String,
        target_type: String,
    },
}

/// Shared compatibility rule for both AST `PortUsage`s and canvas `PositionedPort`s.
pub fn check_port_direction_and_type(
    src_type: Option<&str>,
    src_eff_dir: PortDirection,
    dst_type: Option<&str>,
    dst_eff_dir: PortDirection,
) -> PortCompatibility {
    if let (Some(st), Some(dt)) = (src_type, dst_type) {
        let s_clean = st.trim_start_matches('~');
        let d_clean = dt.trim_start_matches('~');
        if s_clean != d_clean {
            return PortCompatibility::TypeMismatch {
                source_type: s_clean.to_string(),
                target_type: d_clean.to_string(),
            };
        }
    }

    let dir_ok = matches!(
        (src_eff_dir, dst_eff_dir),
        (PortDirection::Out, PortDirection::In)
            | (PortDirection::In, PortDirection::Out)
            | (PortDirection::InOut, _)
            | (_, PortDirection::InOut)
    );
    if dir_ok {
        PortCompatibility::Compatible
    } else {
        PortCompatibility::DirectionMismatch {
            source_dir: src_eff_dir,
            target_dir: dst_eff_dir,
        }
    }
}

/// Contextual occurrence node resolved dynamically from a `DefinitionGraph`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceNode {
    pub path: InstancePath,
    pub label: String,
    pub type_def: Option<ElementId>,
    pub origin: FeatureOrigin,
    pub ports: Vec<PortUsage>,
    #[serde(default)]
    pub attributes: Vec<AttributeUsage>,
    pub children: Vec<InstanceNode>,
    pub span: Span,
}

impl InstanceNode {
    /// Expands a `PartDef` or top-level `PartUsage` into a contextual occurrence tree.
    pub fn expand_from_graph(
        graph: &DefinitionGraph,
        root: &ElementId,
        max_depth: usize,
    ) -> Option<Self> {
        if let Some(def) = graph.resolve_part_def(root) {
            let path = InstancePath::root(def.id.clone());
            return Some(Self::build_from_def(
                graph,
                &path,
                &def.name,
                &def.id,
                FeatureOrigin::Owned,
                def.span.clone(),
                max_depth,
            ));
        }

        let usage = graph.top_level_parts.get(root)?;
        let path = InstancePath::root(usage.id.clone());
        Some(Self::build_from_usage(graph, &path, usage, max_depth))
    }

    fn build_from_def(
        graph: &DefinitionGraph,
        path: &InstancePath,
        label: &str,
        def_id: &ElementId,
        origin: FeatureOrigin,
        span: Span,
        depth_left: usize,
    ) -> Self {
        let ports = graph.effective_ports_for_def(def_id);
        let attributes = graph.effective_attributes_for_def(def_id);
        let children = if depth_left == 0 {
            Vec::new()
        } else {
            let usages = graph.effective_parts_for_def(def_id);
            Self::expand_usages(graph, path, &usages, depth_left - 1)
        };
        Self {
            path: path.clone(),
            label: label.to_string(),
            type_def: Some(def_id.clone()),
            origin,
            ports,
            attributes,
            children,
            span,
        }
    }

    fn build_from_usage(
        graph: &DefinitionGraph,
        path: &InstancePath,
        usage: &PartUsage,
        depth_left: usize,
    ) -> Self {
        let mut ports = usage.ports.clone();
        let mut by_attr: BTreeMap<String, AttributeUsage> = BTreeMap::new();
        let mut children = Vec::new();
        if let Some(ref td) = usage.type_def {
            if ports.is_empty() {
                ports = graph.effective_ports_for_def(td);
            }
            for attr in graph.effective_attributes_for_def(td) {
                by_attr.insert(attr.name.clone(), attr);
            }
            if depth_left > 0 {
                let def_usages = graph.effective_parts_for_def(td);
                children.extend(Self::expand_usages(graph, path, &def_usages, depth_left - 1));
            }
        }
        for local in &usage.attributes {
            let mut merged = local.clone();
            if let Some(prev) = by_attr.get(&merged.name) {
                if merged.type_name.is_none() {
                    merged.type_name = prev.type_name.clone();
                }
                if merged.value_expr.is_none() {
                    merged.value_expr = prev.value_expr.clone();
                }
            }
            by_attr.insert(merged.name.clone(), merged);
        }
        let mut attributes: Vec<AttributeUsage> = by_attr.into_values().collect();
        attributes.sort_by(|a, b| a.span.cmp(&b.span).then_with(|| a.name.cmp(&b.name)));
        if depth_left > 0 && !usage.nested_parts.is_empty() {
            children.extend(Self::expand_usages(
                graph,
                path,
                &usage.nested_parts,
                depth_left - 1,
            ));
        }
        Self {
            path: path.clone(),
            label: usage.name.clone(),
            type_def: usage.type_def.clone(),
            origin: usage.origin.clone(),
            ports,
            attributes,
            children,
            span: usage.span.clone(),
        }
    }

    fn expand_usages(
        graph: &DefinitionGraph,
        parent_path: &InstancePath,
        usages: &[PartUsage],
        depth_left: usize,
    ) -> Vec<Self> {
        let subsetted_targets: BTreeSet<String> = usages
            .iter()
            .filter_map(|u| match &u.origin {
                FeatureOrigin::Subsetted(target) => Some(target.short_name().to_string()),
                _ => None,
            })
            .collect();

        let mut out = Vec::new();
        for usage in usages {
            if let Some(count) = usage.multiplicity {
                if count > 1 && !subsetted_targets.is_empty() {
                    // Explicit subsetting usages (e.g. frontLeftWheel :> wheel) represent the slots.
                    continue;
                }
                if count > 1 {
                    for idx in 1..=count {
                        let indexed_id = ElementId::new(format!("{}#{}", usage.id, idx));
                        let child_path = parent_path.push(indexed_id);
                        let mut node = Self::build_from_usage(graph, &child_path, usage, depth_left);
                        node.label = format!("{}[{}]", usage.name, idx);
                        out.push(node);
                    }
                    continue;
                }
            }

            let child_path = parent_path.push(usage.id.clone());
            out.push(Self::build_from_usage(graph, &child_path, usage, depth_left));
        }
        out
    }
}

/// Evaluates SysML v2 `view` / `expose` scope filters.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewFilter {
    pub expose_patterns: Vec<String>,
    pub include_relationships: Vec<RelationshipKind>,
}

impl ViewFilter {
    pub fn matches_element(&self, id: &ElementId) -> bool {
        if self.expose_patterns.is_empty() {
            return true;
        }
        let qn = id.as_str();
        self.expose_patterns.iter().any(|pattern| {
            if let Some(prefix) = pattern.strip_suffix("::**") {
                qn == prefix
                    || qn.starts_with(&format!("{prefix}::"))
                    || qn.ends_with(&format!("::{prefix}"))
                    || qn.contains(&format!("::{prefix}::"))
            } else if let Some(prefix) = pattern.strip_suffix("::*") {
                let direct_root = qn
                    .strip_prefix(&format!("{prefix}::"))
                    .is_some_and(|rest| !rest.contains("::"));
                let direct_relative = qn
                    .rsplit_once(&format!("::{prefix}::"))
                    .is_some_and(|(_, rest)| !rest.contains("::"));
                direct_root || direct_relative
            } else {
                qn == pattern || qn.ends_with(&format!("::{pattern}"))
            }
        })
    }

    pub fn allows_relationship(&self, kind: RelationshipKind) -> bool {
        self.include_relationships.is_empty() || self.include_relationships.contains(&kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conjugated_port_flips_direction() {
        assert_eq!(PortDirection::Out.effective_direction(false), PortDirection::Out);
        assert_eq!(PortDirection::Out.effective_direction(true), PortDirection::In);
        assert_eq!(PortDirection::In.effective_direction(true), PortDirection::Out);
        assert_eq!(PortDirection::InOut.effective_direction(true), PortDirection::InOut);
    }

    #[test]
    fn instance_tree_expands_multiplicity_and_keeps_paths_distinct() {
        let mut graph = DefinitionGraph::default();
        let wheel_def = ElementId::new("ChassisPkg::Wheel");
        graph.part_defs.insert(
            wheel_def.clone(),
            PartDef {
                id: wheel_def.clone(),
                name: "Wheel".into(),
                specializes: vec![],
                ports: vec![],
                parts: vec![],
                attributes: vec![],
                attribute_usages: vec![],
                span: Span::new("chassis.sysml", 10, 40),
            },
        );
        let chassis_def = ElementId::new("ChassisPkg::Chassis");
        graph.part_defs.insert(
            chassis_def.clone(),
            PartDef {
                id: chassis_def.clone(),
                name: "Chassis".into(),
                specializes: vec![],
                ports: vec![],
                parts: vec![PartUsage {
                    id: ElementId::new("ChassisPkg::Chassis::wheel"),
                    name: "wheel".into(),
                    type_def: Some(wheel_def),
                    multiplicity: Some(4),
                    origin: FeatureOrigin::Owned,
                    ports: vec![],
                    nested_parts: vec![],
                    attributes: vec![],
                    span: Span::new("chassis.sysml", 60, 85),
                }],
                attributes: vec![],
                attribute_usages: vec![],
                span: Span::new("chassis.sysml", 50, 100),
            },
        );

        let tree = InstanceNode::expand_from_graph(&graph, &chassis_def, 3).unwrap();
        assert_eq!(tree.children.len(), 4);
        assert_eq!(tree.children[0].label, "wheel[1]");
        assert_eq!(tree.children[3].label, "wheel[4]");
        assert_ne!(
            tree.children[0].path.canonical_key(),
            tree.children[3].path.canonical_key()
        );
    }

    #[test]
    fn view_filter_matches_recursive_and_direct_expose() {
        let filter = ViewFilter {
            expose_patterns: vec!["Power::**".into(), "RoverSystem::*".into()],
            include_relationships: vec![RelationshipKind::Connection],
        };
        assert!(filter.matches_element(&ElementId::new("Power::BatteryPack::powerOut")));
        assert!(filter.matches_element(&ElementId::new("RoverSystem::Rover")));
        assert!(!filter.matches_element(&ElementId::new("RoverSystem::Rover::chassis")));
        assert!(filter.allows_relationship(RelationshipKind::Connection));
        assert!(!filter.allows_relationship(RelationshipKind::Satisfy));

        let rel_filter = ViewFilter {
            expose_patterns: vec!["PartsTree::**".into()],
            include_relationships: vec![],
        };
        assert!(rel_filter.matches_element(&ElementId::new(
            "SimpleVehicleModel::VehicleConfigurations::VehicleConfiguration_b::PartsTree::vehicle_b"
        )));
        let direct_rel = ViewFilter {
            expose_patterns: vec!["PartsTree::*".into()],
            include_relationships: vec![],
        };
        assert!(direct_rel.matches_element(&ElementId::new(
            "SimpleVehicleModel::VehicleConfigurations::VehicleConfiguration_b::PartsTree::vehicle_b"
        )));
        assert!(!direct_rel.matches_element(&ElementId::new(
            "SimpleVehicleModel::VehicleConfigurations::VehicleConfiguration_b::PartsTree::vehicle_b::engine"
        )));
    }

    #[test]
    fn port_compatibility_checks_type_and_conjugated_direction() {
        let mut graph = DefinitionGraph::default();
        let power_def = ElementId::new("Power::PowerPort");
        let torque_def = ElementId::new("ChassisPkg::DriveTorquePort");
        graph.port_defs.insert(
            power_def.clone(),
            PortDef {
                id: power_def.clone(),
                name: "PowerPort".into(),
                direction: PortDirection::Out,
                item_type: Some("PowerFlow".into()),
                span: Span::default(),
            },
        );
        graph.port_defs.insert(
            torque_def.clone(),
            PortDef {
                id: torque_def.clone(),
                name: "DriveTorquePort".into(),
                direction: PortDirection::Out,
                item_type: Some("Real".into()),
                span: Span::default(),
            },
        );

        let p_out = PortUsage {
            id: ElementId::new("pOut"),
            name: "powerOut".into(),
            port_def: Some(power_def.clone()),
            is_conjugated: false,
            direction: PortDirection::Out,
            origin: FeatureOrigin::Owned,
            span: Span::default(),
        };
        let p_in_conj = PortUsage {
            id: ElementId::new("pIn"),
            name: "powerIn".into(),
            port_def: Some(power_def.clone()),
            is_conjugated: true,
            direction: PortDirection::In,
            origin: FeatureOrigin::Owned,
            span: Span::default(),
        };
        let p_out_unconj = PortUsage {
            is_conjugated: false,
            direction: PortDirection::Out,
            ..p_in_conj.clone()
        };
        let torque_in = PortUsage {
            id: ElementId::new("tIn"),
            name: "driveIn".into(),
            port_def: Some(torque_def),
            is_conjugated: true,
            direction: PortDirection::In,
            origin: FeatureOrigin::Owned,
            span: Span::default(),
        };

        assert_eq!(
            graph.check_port_compatibility(&p_out, &p_in_conj),
            PortCompatibility::Compatible
        );
        assert_eq!(
            graph.check_port_compatibility(&p_out, &p_out_unconj),
            PortCompatibility::DirectionMismatch {
                source_dir: PortDirection::Out,
                target_dir: PortDirection::Out,
            }
        );
        assert_eq!(
            graph.check_port_compatibility(&p_out, &torque_in),
            PortCompatibility::TypeMismatch {
                source_type: "PowerPort".into(),
                target_type: "DriveTorquePort".into(),
            }
        );
    }

    #[test]
    fn resolve_action_matches_exact_and_short_name() {
        let mut graph = DefinitionGraph::default();
        let act_id = ElementId::new("System Behavior::Visitor Arrival Story");
        graph.actions.insert(
            act_id.clone(),
            ActionNode {
                id: act_id.clone(),
                name: "Visitor Arrival Story".into(),
                kind: ActionNodeKind::Action,
                is_def: false,
                type_def: None,
                params: vec![],
                doc: None,
                children: vec![],
                span: Span::new("behavior.sysml", 0, 20),
            },
        );
        assert_eq!(graph.resolve_action(&act_id).map(|a| &a.id), Some(&act_id));
        assert_eq!(
            graph
                .resolve_action(&ElementId::new("Visitor Arrival Story"))
                .map(|a| &a.id),
            Some(&act_id)
        );
        assert_eq!(ViewKind::default(), ViewKind::General);
    }
}
