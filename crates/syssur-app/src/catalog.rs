//! Unified OMG SysML v2 (formal/26-03-02 Clause 7) catalog of definitions,
//! usages, behavioral nodes, states, cases, views, annotations, and relationships.
//!
//! Shared across the right-click diagram context menu, the Model Explorer creation
//! menus, and the optional toggleable palette drawer.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2};
use syssur_layout::NodeShape;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateContext {
    /// Diagram Palette drawer (standalone graphical shapes + connectors/relationships).
    DiagramPalette,
    /// Right-click on empty diagram canvas (standalone graphical shapes only).
    EmptyCanvas,
    /// Right-click "Create inside..." on a specific diagram node shape.
    InsideNode(NodeShape),
    /// Right-click "Create in Package..." in the Model Explorer (full package-level elements).
    PackageExplorer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CatalogCategory {
    NamespacesAndAnnotations,
    Structure,
    PortsAndInterfaces,
    ActionsAndControl,
    StatesAndTransitions,
    CalcConstraintsRequirements,
    CasesAndVerification,
    ViewsAndMetadata,
    Relationships,
}

impl CatalogCategory {
    pub const ALL_CREATE: [Self; 8] = [
        Self::NamespacesAndAnnotations,
        Self::Structure,
        Self::PortsAndInterfaces,
        Self::ActionsAndControl,
        Self::StatesAndTransitions,
        Self::CalcConstraintsRequirements,
        Self::CasesAndVerification,
        Self::ViewsAndMetadata,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NamespacesAndAnnotations => "Namespaces & Annotations (§7.3-7.5)",
            Self::Structure => "Structure (§7.7-7.11)",
            Self::PortsAndInterfaces => "Ports & Interfaces (§7.12-7.16)",
            Self::ActionsAndControl => "Actions & Control (§7.17)",
            Self::StatesAndTransitions => "States & Transitions (§7.18)",
            Self::CalcConstraintsRequirements => "Calc, Constraints & Req (§7.19-7.21)",
            Self::CasesAndVerification => "Cases & Verification (§7.22-7.25)",
            Self::ViewsAndMetadata => "Views & Metadata (§7.26-7.27)",
            Self::Relationships => "Relationships & Connectors",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysmlCatalogItem {
    pub id: &'static str,
    pub label: &'static str,
    pub keyword: &'static str,
    pub is_def: bool,
    pub is_relationship: bool,
    pub category: CatalogCategory,
    pub default_name: &'static str,
    pub doc_hint: &'static str,
}

pub const SYSML_V2_CATALOG: &[SysmlCatalogItem] = &[
    // §7.3-7.5 Namespaces & Annotations
    SysmlCatalogItem {
        id: "pkg",
        label: "Package",
        keyword: "package",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "NewPackage",
        doc_hint: "SysML v2 package namespace",
    },
    SysmlCatalogItem {
        id: "lib_pkg",
        label: "Library Package",
        keyword: "library package",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "DomainLibrary",
        doc_hint: "Reusable standard/domain library package",
    },
    SysmlCatalogItem {
        id: "import",
        label: "Import (::*)",
        keyword: "import",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "ScalarValues",
        doc_hint: "Namespace import",
    },
    SysmlCatalogItem {
        id: "alias",
        label: "Alias",
        keyword: "alias",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "AliasName",
        doc_hint: "Alias for target element",
    },
    SysmlCatalogItem {
        id: "comment",
        label: "Comment",
        keyword: "comment",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "note",
        doc_hint: "Model annotation comment",
    },
    SysmlCatalogItem {
        id: "filter",
        label: "Element Filter",
        keyword: "filter",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::NamespacesAndAnnotations,
        default_name: "true",
        doc_hint: "Package/view filter expression",
    },
    // §7.7-7.11 Structure
    SysmlCatalogItem {
        id: "part_def",
        label: "Part Definition",
        keyword: "part",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "NewPartDef",
        doc_hint: "Structural part definition",
    },
    SysmlCatalogItem {
        id: "part",
        label: "Part Usage",
        keyword: "part",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "newPart",
        doc_hint: "Contextual part usage",
    },
    SysmlCatalogItem {
        id: "attr_def",
        label: "Attribute Definition",
        keyword: "attribute",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "NewAttrDef",
        doc_hint: "Attribute value type definition",
    },
    SysmlCatalogItem {
        id: "attr",
        label: "Attribute Usage",
        keyword: "attribute",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "attr",
        doc_hint: "Attribute feature",
    },
    SysmlCatalogItem {
        id: "enum_def",
        label: "Enumeration Def",
        keyword: "enum",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "ModeEnum",
        doc_hint: "Enumeration definition",
    },
    SysmlCatalogItem {
        id: "item_def",
        label: "Item Definition",
        keyword: "item",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "PayloadDef",
        doc_hint: "Flowing/transferable item definition",
    },
    SysmlCatalogItem {
        id: "item",
        label: "Item Usage",
        keyword: "item",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "payload",
        doc_hint: "Item usage",
    },
    SysmlCatalogItem {
        id: "occ_def",
        label: "Occurrence Definition",
        keyword: "occurrence",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "EventDef",
        doc_hint: "4D occurrence definition",
    },
    SysmlCatalogItem {
        id: "occ",
        label: "Occurrence Usage",
        keyword: "occurrence",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "occ",
        doc_hint: "Occurrence usage",
    },
    SysmlCatalogItem {
        id: "event",
        label: "Event Occurrence",
        keyword: "event",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "ev",
        doc_hint: "Event occurrence usage",
    },
    SysmlCatalogItem {
        id: "timeslice",
        label: "Timeslice",
        keyword: "timeslice",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "phase",
        doc_hint: "Temporal timeslice portion",
    },
    SysmlCatalogItem {
        id: "snapshot",
        label: "Snapshot",
        keyword: "snapshot",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "snap",
        doc_hint: "Instantaneous snapshot portion",
    },
    SysmlCatalogItem {
        id: "ref",
        label: "Reference Usage (ref)",
        keyword: "ref",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::Structure,
        default_name: "refFeature",
        doc_hint: "Non-composite reference usage",
    },
    // §7.12-7.16 Ports & Interfaces
    SysmlCatalogItem {
        id: "port_def",
        label: "Port Definition",
        keyword: "port",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "NewPortDef",
        doc_hint: "Port definition",
    },
    SysmlCatalogItem {
        id: "port",
        label: "Port Usage",
        keyword: "port",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "port",
        doc_hint: "Boundary port usage",
    },
    SysmlCatalogItem {
        id: "conj_port",
        label: "Conjugated Port (~)",
        keyword: "~port",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "conjPort",
        doc_hint: "Conjugated consumer port",
    },
    SysmlCatalogItem {
        id: "iface_def",
        label: "Interface Definition",
        keyword: "interface",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "NewInterfaceDef",
        doc_hint: "Typed interface definition",
    },
    SysmlCatalogItem {
        id: "iface",
        label: "Interface Usage",
        keyword: "interface",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "iface",
        doc_hint: "Interface usage",
    },
    SysmlCatalogItem {
        id: "conn_def",
        label: "Connection Definition",
        keyword: "connection",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "NewConnectionDef",
        doc_hint: "Connection definition",
    },
    SysmlCatalogItem {
        id: "alloc_def",
        label: "Allocation Definition",
        keyword: "allocation",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "NewAllocationDef",
        doc_hint: "Allocation definition",
    },
    SysmlCatalogItem {
        id: "flow_def",
        label: "Flow Definition",
        keyword: "flow",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::PortsAndInterfaces,
        default_name: "NewFlowDef",
        doc_hint: "Flow definition",
    },
    // §7.17 Actions & Control
    SysmlCatalogItem {
        id: "action_def",
        label: "Action Definition",
        keyword: "action",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "NewActionDef",
        doc_hint: "Action behavior definition",
    },
    SysmlCatalogItem {
        id: "action",
        label: "Action Usage",
        keyword: "action",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "step",
        doc_hint: "Action step usage",
    },
    SysmlCatalogItem {
        id: "perform_action",
        label: "Perform Action",
        keyword: "perform action",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "performedStep",
        doc_hint: "Perform action reference",
    },
    SysmlCatalogItem {
        id: "start",
        label: "Start Node (first start)",
        keyword: "start",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "start",
        doc_hint: "Initial action flow node",
    },
    SysmlCatalogItem {
        id: "done",
        label: "Done Node",
        keyword: "done",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "done",
        doc_hint: "Final action node",
    },
    SysmlCatalogItem {
        id: "fork",
        label: "Fork Node",
        keyword: "fork",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "forkNode",
        doc_hint: "Parallel fork control node",
    },
    SysmlCatalogItem {
        id: "join",
        label: "Join Node",
        keyword: "join",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "joinNode",
        doc_hint: "Parallel join control node",
    },
    SysmlCatalogItem {
        id: "decide",
        label: "Decide Node",
        keyword: "decide",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "decideNode",
        doc_hint: "Decision branch control node",
    },
    SysmlCatalogItem {
        id: "merge",
        label: "Merge Node",
        keyword: "merge",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "mergeNode",
        doc_hint: "Merge control node",
    },
    SysmlCatalogItem {
        id: "accept",
        label: "Accept Action",
        keyword: "accept",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "sig",
        doc_hint: "Accept trigger/payload action",
    },
    SysmlCatalogItem {
        id: "send",
        label: "Send Action",
        keyword: "send",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "signal",
        doc_hint: "Send payload action",
    },
    SysmlCatalogItem {
        id: "assign",
        label: "Assign Action",
        keyword: "assign",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "flag",
        doc_hint: "Feature assignment action",
    },
    SysmlCatalogItem {
        id: "terminate",
        label: "Terminate Action",
        keyword: "terminate",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "terminate",
        doc_hint: "Terminate execution node",
    },
    SysmlCatalogItem {
        id: "if_node",
        label: "If Action",
        keyword: "if",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "thenStep",
        doc_hint: "Conditional if-then action",
    },
    SysmlCatalogItem {
        id: "while_node",
        label: "While / Loop Action",
        keyword: "while",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "loopBody",
        doc_hint: "Loop action node",
    },
    SysmlCatalogItem {
        id: "for_node",
        label: "For Loop Action",
        keyword: "for",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ActionsAndControl,
        default_name: "iterBody",
        doc_hint: "For-each loop action",
    },
    // §7.18 States & Transitions
    SysmlCatalogItem {
        id: "state_def",
        label: "State Definition",
        keyword: "state",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::StatesAndTransitions,
        default_name: "StateMachineDef",
        doc_hint: "State machine definition",
    },
    SysmlCatalogItem {
        id: "state",
        label: "State Usage",
        keyword: "state",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::StatesAndTransitions,
        default_name: "State",
        doc_hint: "State usage",
    },
    SysmlCatalogItem {
        id: "exhibit_state",
        label: "Exhibit State",
        keyword: "exhibit state",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::StatesAndTransitions,
        default_name: "modes",
        doc_hint: "Exhibit state machine on part",
    },
    // §7.19-7.21 Calculations, Constraints & Requirements
    SysmlCatalogItem {
        id: "calc_def",
        label: "Calc Definition",
        keyword: "calc",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "NewCalcDef",
        doc_hint: "Calculation definition",
    },
    SysmlCatalogItem {
        id: "calc",
        label: "Calc Usage",
        keyword: "calc",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "compute",
        doc_hint: "Calculation usage",
    },
    SysmlCatalogItem {
        id: "constraint_def",
        label: "Constraint Definition",
        keyword: "constraint",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "NewConstraintDef",
        doc_hint: "Constraint definition",
    },
    SysmlCatalogItem {
        id: "constraint",
        label: "Constraint Usage",
        keyword: "constraint",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "constraint1",
        doc_hint: "Constraint usage",
    },
    SysmlCatalogItem {
        id: "assert_constraint",
        label: "Assert Constraint",
        keyword: "assert constraint",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "invariant",
        doc_hint: "Asserted constraint",
    },
    SysmlCatalogItem {
        id: "req_def",
        label: "Requirement Definition",
        keyword: "requirement",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "NewReqDef",
        doc_hint: "Reusable requirement definition",
    },
    SysmlCatalogItem {
        id: "req",
        label: "Requirement Usage",
        keyword: "requirement",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "NewRequirement",
        doc_hint: "Concrete requirement usage",
    },
    SysmlCatalogItem {
        id: "concern_def",
        label: "Concern Definition",
        keyword: "concern",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "NewConcernDef",
        doc_hint: "Stakeholder concern definition",
    },
    SysmlCatalogItem {
        id: "concern",
        label: "Concern Usage",
        keyword: "concern",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CalcConstraintsRequirements,
        default_name: "concern1",
        doc_hint: "Stakeholder concern usage",
    },
    // §7.22-7.25 Cases & Verification
    SysmlCatalogItem {
        id: "case_def",
        label: "Case Definition",
        keyword: "case",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "NewCaseDef",
        doc_hint: "Generic case definition",
    },
    SysmlCatalogItem {
        id: "use_case_def",
        label: "Use Case Definition",
        keyword: "use case",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "NewUseCaseDef",
        doc_hint: "Use case definition",
    },
    SysmlCatalogItem {
        id: "use_case",
        label: "Use Case Usage",
        keyword: "use case",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "useCase",
        doc_hint: "Use case usage",
    },
    SysmlCatalogItem {
        id: "include_uc",
        label: "Include Use Case",
        keyword: "include use case",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "includedUC",
        doc_hint: "Include use case",
    },
    SysmlCatalogItem {
        id: "analysis_def",
        label: "Analysis Case Def",
        keyword: "analysis",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "TradeAnalysis",
        doc_hint: "Analysis case definition",
    },
    SysmlCatalogItem {
        id: "verif_def",
        label: "Verification Case Def",
        keyword: "verification",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "VerifySystem",
        doc_hint: "Verification case definition",
    },
    SysmlCatalogItem {
        id: "subject",
        label: "Subject",
        keyword: "subject",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "system",
        doc_hint: "Case/requirement subject",
    },
    SysmlCatalogItem {
        id: "actor",
        label: "Actor",
        keyword: "actor",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "operator",
        doc_hint: "Use case actor",
    },
    SysmlCatalogItem {
        id: "stakeholder",
        label: "Stakeholder",
        keyword: "stakeholder",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::CasesAndVerification,
        default_name: "owner",
        doc_hint: "Viewpoint/requirement stakeholder",
    },
    // §7.26-7.27 Views & Metadata
    SysmlCatalogItem {
        id: "general_view",
        label: "General View",
        keyword: "view:GeneralView",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "GeneralOverview",
        doc_hint: "General SysML v2 view",
    },
    SysmlCatalogItem {
        id: "interconn_view",
        label: "Interconnection View",
        keyword: "view:InterconnectionView",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "Interconnection",
        doc_hint: "Internal interconnection view",
    },
    SysmlCatalogItem {
        id: "action_view",
        label: "Action Flow View",
        keyword: "view:ActionFlowView",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "ActionFlow",
        doc_hint: "Action flow view",
    },
    SysmlCatalogItem {
        id: "state_view",
        label: "State Transition View",
        keyword: "view:StateTransitionView",
        is_def: false,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "StateMachine",
        doc_hint: "State transition view",
    },
    SysmlCatalogItem {
        id: "viewpoint_def",
        label: "Viewpoint Definition",
        keyword: "viewpoint",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "SafetyViewpoint",
        doc_hint: "Viewpoint definition",
    },
    SysmlCatalogItem {
        id: "rendering_def",
        label: "Rendering Definition",
        keyword: "rendering",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "CustomRender",
        doc_hint: "View rendering definition",
    },
    SysmlCatalogItem {
        id: "metadata_def",
        label: "Metadata Definition",
        keyword: "metadata",
        is_def: true,
        is_relationship: false,
        category: CatalogCategory::ViewsAndMetadata,
        default_name: "CustomMeta",
        doc_hint: "User metadata definition",
    },
    // Relationships & Connectors
    SysmlCatalogItem {
        id: "rel_connection",
        label: "Connection (connect)",
        keyword: "connection",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "connect",
        doc_hint: "Connect two parts/ports",
    },
    SysmlCatalogItem {
        id: "rel_interface",
        label: "Interface Connection",
        keyword: "interface",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "interface",
        doc_hint: "Typed interface link",
    },
    SysmlCatalogItem {
        id: "rel_binding",
        label: "Binding (bind =)",
        keyword: "binding",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "bind",
        doc_hint: "Equality binding connector",
    },
    SysmlCatalogItem {
        id: "rel_flow",
        label: "Flow (flow from..to)",
        keyword: "flow",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "flow",
        doc_hint: "Item flow between features",
    },
    SysmlCatalogItem {
        id: "rel_succession",
        label: "Succession (first..then)",
        keyword: "succession",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "succession",
        doc_hint: "Control/temporal succession",
    },
    SysmlCatalogItem {
        id: "rel_guarded_succ",
        label: "Guarded Succession (if)",
        keyword: "guarded_succession",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "guarded",
        doc_hint: "Conditional guarded succession",
    },
    SysmlCatalogItem {
        id: "rel_transition",
        label: "State Transition",
        keyword: "transition",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "transition",
        doc_hint: "State transition edge",
    },
    SysmlCatalogItem {
        id: "rel_specialize",
        label: "Specialization (:>)",
        keyword: "specialization",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: ":>",
        doc_hint: "Generalization / specialization",
    },
    SysmlCatalogItem {
        id: "rel_redefine",
        label: "Redefinition (:>>)",
        keyword: "redefinition",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: ":>>",
        doc_hint: "Feature redefinition",
    },
    SysmlCatalogItem {
        id: "rel_typing",
        label: "Feature Typing (:)",
        keyword: "typing",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: ":",
        doc_hint: "Type usage by definition",
    },
    SysmlCatalogItem {
        id: "rel_satisfy",
        label: "Satisfy Requirement",
        keyword: "satisfy",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "satisfy",
        doc_hint: "Satisfy requirement by element",
    },
    SysmlCatalogItem {
        id: "rel_verify",
        label: "Verify Requirement",
        keyword: "verify",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "verify",
        doc_hint: "Verification case verifies requirement",
    },
    SysmlCatalogItem {
        id: "rel_allocate",
        label: "Allocate (allocate..to)",
        keyword: "allocate",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "allocate",
        doc_hint: "Allocate source to target",
    },
    SysmlCatalogItem {
        id: "rel_dependency",
        label: "Dependency (from..to)",
        keyword: "dependency",
        is_def: false,
        is_relationship: true,
        category: CatalogCategory::Relationships,
        default_name: "dependency",
        doc_hint: "Client-to-supplier dependency",
    },
];

pub fn catalog_relationships() -> impl Iterator<Item = &'static SysmlCatalogItem> {
    SYSML_V2_CATALOG.iter().filter(|item| item.is_relationship)
}

/// Returns whether a catalog item should be shown in the given UI creation context.
pub fn is_allowed_in_context(item: &SysmlCatalogItem, ctx: CreateContext) -> bool {
    if item.is_relationship {
        return matches!(ctx, CreateContext::DiagramPalette);
    }

    match ctx {
        CreateContext::PackageExplorer => true,
        CreateContext::DiagramPalette | CreateContext::EmptyCanvas => matches!(
            item.id,
            "pkg"
                | "comment"
                | "part_def"
                | "part"
                | "item_def"
                | "item"
                | "occ_def"
                | "occ"
                | "enum_def"
                | "port_def"
                | "iface_def"
                | "conn_def"
                | "alloc_def"
                | "flow_def"
                | "action_def"
                | "action"
                | "start"
                | "done"
                | "fork"
                | "join"
                | "decide"
                | "merge"
                | "accept"
                | "send"
                | "assign"
                | "terminate"
                | "if_node"
                | "while_node"
                | "for_node"
                | "state_def"
                | "state"
                | "calc_def"
                | "calc"
                | "constraint_def"
                | "constraint"
                | "req_def"
                | "req"
                | "concern_def"
                | "concern"
                | "case_def"
                | "use_case_def"
                | "use_case"
                | "analysis_def"
                | "verif_def"
        ),
        CreateContext::InsideNode(shape) => match shape {
            NodeShape::PartBlock => matches!(
                item.id,
                "attr"
                    | "port"
                    | "conj_port"
                    | "part"
                    | "item"
                    | "ref"
                    | "perform_action"
                    | "exhibit_state"
                    | "assert_constraint"
                    | "constraint"
                    | "calc"
                    | "req"
                    | "comment"
            ),
            NodeShape::ActionBox => matches!(
                item.id,
                "action"
                    | "start"
                    | "done"
                    | "fork"
                    | "join"
                    | "decide"
                    | "merge"
                    | "accept"
                    | "send"
                    | "assign"
                    | "terminate"
                    | "if_node"
                    | "while_node"
                    | "for_node"
                    | "attr"
                    | "comment"
            ),
            NodeShape::StateBox => matches!(item.id, "state" | "attr" | "comment"),
            NodeShape::RequirementCard => matches!(
                item.id,
                "req" | "constraint" | "assert_constraint" | "subject" | "stakeholder" | "attr" | "comment"
            ),
            NodeShape::ConstraintCard => matches!(item.id, "attr" | "constraint" | "comment"),
            NodeShape::CaseOval => matches!(
                item.id,
                "subject" | "actor" | "stakeholder" | "include_uc" | "use_case" | "action" | "req" | "comment"
            ),
            NodeShape::StartCircle
            | NodeShape::DoneBullseye
            | NodeShape::ForkJoinBar
            | NodeShape::DecideDiamond
            | NodeShape::MergeDiamond => false,
            NodeShape::GenericElement => {
                matches!(item.id, "attr" | "port" | "conj_port" | "part" | "item" | "comment")
            }
        },
    }
}

/// Returns true if at least one catalog item is allowed in `ctx`.
pub fn has_items_for_context(ctx: CreateContext) -> bool {
    SYSML_V2_CATALOG
        .iter()
        .any(|item| is_allowed_in_context(item, ctx))
}

fn category_accent(cat: CatalogCategory, keyword: &str) -> Color32 {
    match cat {
        CatalogCategory::NamespacesAndAnnotations => Color32::from_rgb(160, 175, 200),
        CatalogCategory::Structure => Color32::from_rgb(125, 185, 240),
        CatalogCategory::PortsAndInterfaces => {
            if keyword == "~port" {
                Color32::from_rgb(245, 165, 95)
            } else {
                Color32::from_rgb(110, 205, 245)
            }
        }
        CatalogCategory::ActionsAndControl => Color32::from_rgb(92, 195, 185),
        CatalogCategory::StatesAndTransitions => Color32::from_rgb(180, 140, 245),
        CatalogCategory::CalcConstraintsRequirements => Color32::from_rgb(235, 185, 85),
        CatalogCategory::CasesAndVerification => Color32::from_rgb(145, 185, 245),
        CatalogCategory::ViewsAndMetadata => Color32::from_rgb(165, 175, 225),
        CatalogCategory::Relationships => Color32::from_rgb(135, 210, 175),
    }
}

/// Paints a compact SysML v2 graphical icon inside `rect` (recommended 13x13 or 14x14).
pub fn paint_catalog_icon(painter: &egui::Painter, rect: Rect, item: &SysmlCatalogItem) {
    paint_kind_icon(
        painter,
        rect,
        item.category,
        item.keyword,
        item.is_def,
        item.is_relationship,
    );
}

/// Paints a compact SysML v2 graphical icon for any element category/keyword/definition flag.
pub fn paint_kind_icon(
    painter: &egui::Painter,
    rect: Rect,
    category: CatalogCategory,
    keyword: &str,
    is_def: bool,
    is_relationship: bool,
) {
    let accent = category_accent(category, keyword);
    let fill = Color32::from_rgb(30, 36, 46);
    let stroke = Stroke::new(1.2, accent);
    let c = rect.center();

    if is_relationship {
        let p0 = Pos2::new(rect.left() + 1.5, rect.bottom() - 2.5);
        let p1 = Pos2::new(rect.right() - 1.5, rect.top() + 2.5);
        let dashed = matches!(
            keyword,
            "satisfy" | "verify" | "dependency" | "succession" | "guarded_succession"
        );
        if dashed {
            let mid = Pos2::new((p0.x + p1.x) * 0.5, (p0.y + p1.y) * 0.5);
            painter.line_segment([p0, p0.lerp(mid, 0.7)], stroke);
            painter.line_segment([mid.lerp(p1, 0.2), p1], stroke);
        } else {
            painter.line_segment([p0, p1], stroke);
        }
        let dir = (p1 - p0).normalized();
        let perp = Vec2::new(-dir.y, dir.x);
        match keyword {
            "connection" | "interface" => {
                painter.circle_filled(p0, 1.8, accent);
                painter.circle_filled(p1, 1.8, accent);
            }
            "binding" => {
                painter.text(
                    c,
                    Align2::CENTER_CENTER,
                    "=",
                    FontId::monospace(10.5),
                    accent,
                );
            }
            "specialization" => {
                let a = p1 - dir * 5.0 + perp * 3.0;
                let b = p1 - dir * 5.0 - perp * 3.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![p1, a, b],
                    Color32::from_rgb(20, 24, 30),
                    Stroke::new(1.1, accent),
                ));
            }
            "flow" => {
                let a = p1 - dir * 5.0 + perp * 3.0;
                let b = p1 - dir * 5.0 - perp * 3.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![p1, a, b],
                    accent,
                    Stroke::NONE,
                ));
            }
            _ => {
                let a = p1 - dir * 4.5 + perp * 3.0;
                let b = p1 - dir * 4.5 - perp * 3.0;
                painter.line_segment([p1, a], stroke);
                painter.line_segment([p1, b], stroke);
            }
        }
        return;
    }

    match keyword {
        "package" | "library package" => {
            let tab = Rect::from_min_max(
                Pos2::new(rect.left() + 1.5, rect.top() + 2.0),
                Pos2::new(rect.left() + 7.0, rect.top() + 4.5),
            );
            let body = Rect::from_min_max(
                Pos2::new(rect.left() + 1.5, rect.top() + 4.0),
                Pos2::new(rect.right() - 1.5, rect.bottom() - 2.0),
            );
            painter.rect_filled(tab, CornerRadius::ZERO, accent.linear_multiply(0.35));
            painter.rect_stroke(tab, CornerRadius::ZERO, stroke, StrokeKind::Inside);
            painter.rect_filled(body, CornerRadius::ZERO, fill);
            painter.rect_stroke(body, CornerRadius::ZERO, stroke, StrokeKind::Inside);
        }
        "port" | "~port" => {
            let r = rect.shrink(2.5);
            painter.rect_filled(r, CornerRadius::same(1), fill);
            painter.rect_stroke(r, CornerRadius::same(1), stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(c.x - 2.5, c.y), Pos2::new(c.x + 2.5, c.y)],
                stroke,
            );
            if keyword == "~port" {
                painter.line_segment(
                    [Pos2::new(c.x - 2.0, c.y), Pos2::new(c.x, c.y - 2.0)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x - 2.0, c.y), Pos2::new(c.x, c.y + 2.0)],
                    stroke,
                );
            } else {
                painter.line_segment(
                    [Pos2::new(c.x + 2.0, c.y), Pos2::new(c.x, c.y - 2.0)],
                    stroke,
                );
                painter.line_segment(
                    [Pos2::new(c.x + 2.0, c.y), Pos2::new(c.x, c.y + 2.0)],
                    stroke,
                );
            }
        }
        "start" => {
            painter.circle_filled(c, 4.5, Color32::from_rgb(230, 238, 248));
        }
        "done" => {
            painter.circle_stroke(c, 5.0, Stroke::new(1.3, Color32::from_rgb(230, 238, 248)));
            painter.circle_filled(c, 2.7, Color32::from_rgb(230, 238, 248));
        }
        "terminate" => {
            painter.line_segment(
                [rect.left_top() + Vec2::splat(3.0), rect.right_bottom() - Vec2::splat(3.0)],
                Stroke::new(1.5, accent),
            );
            painter.line_segment(
                [rect.right_top() + Vec2::new(-3.0, 3.0), rect.left_bottom() + Vec2::new(3.0, -3.0)],
                Stroke::new(1.5, accent),
            );
        }
        "fork" | "join" => {
            let bar = Rect::from_center_size(c, Vec2::new(rect.width() - 2.0, 3.0));
            painter.rect_filled(bar, 1.0, Color32::from_rgb(225, 234, 246));
        }
        "decide" | "merge" => {
            let r = 5.2;
            let pts = vec![
                Pos2::new(c.x, c.y - r),
                Pos2::new(c.x + r, c.y),
                Pos2::new(c.x, c.y + r),
                Pos2::new(c.x - r, c.y),
            ];
            let dfill = if keyword == "merge" {
                Color32::from_rgb(195, 215, 240)
            } else {
                fill
            };
            painter.add(egui::Shape::convex_polygon(pts, dfill, stroke));
        }
        "use case" | "analysis" | "verification" | "case" => {
            let r = rect.shrink2(Vec2::new(1.0, 2.5));
            painter.rect_filled(r, CornerRadius::same(6), fill);
            painter.rect_stroke(r, CornerRadius::same(6), stroke, StrokeKind::Inside);
        }
        "comment" | "alias" | "import" | "filter" => {
            let ch = match keyword {
                "comment" => "/*",
                "alias" => "::",
                "import" => "->",
                _ => "[]",
            };
            painter.text(c, Align2::CENTER_CENTER, ch, FontId::monospace(9.5), accent);
        }
        _ => {
            let r = rect.shrink2(Vec2::new(1.5, 2.0));
            let cr = if is_def {
                CornerRadius::ZERO
            } else {
                CornerRadius::same(3)
            };
            painter.rect_filled(r, cr, fill);
            painter.rect_stroke(r, cr, stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(r.left(), r.top() + 4.0), Pos2::new(r.right(), r.top() + 4.0)],
                Stroke::new(1.0, accent.linear_multiply(0.7)),
            );
        }
    }
}

/// Interactive catalog item row showing a SysML v2 vector icon + label (+ optional DnD payload).
pub fn catalog_item_row(
    ui: &mut Ui,
    item: &'static SysmlCatalogItem,
    selected: bool,
    draggable: bool,
) -> egui::Response {
    let height = 20.0;
    let width = ui.available_width().max(150.0);
    let sense = if draggable {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), sense);

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&resp, selected);
        if selected || resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 3.0, visuals.bg_fill);
        }
        let icon_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 4.0, rect.center().y - 7.0),
            Vec2::splat(14.0),
        );
        paint_catalog_icon(ui.painter(), icon_rect, item);
        ui.painter().text(
            Pos2::new(rect.left() + 23.0, rect.center().y),
            Align2::LEFT_CENTER,
            item.label,
            FontId::proportional(12.5),
            visuals.text_color(),
        );
    }

    if draggable {
        resp.dnd_set_drag_payload(item);
    }
    resp.on_hover_text(item.doc_hint)
}

/// Renders clean, non-convoluted Clause 7 creation submenus (or a flat filtered list when searching),
/// curated according to `ctx`.
pub fn show_create_submenus(
    ui: &mut Ui,
    search_query: &str,
    ctx: CreateContext,
) -> Option<&'static SysmlCatalogItem> {
    let q = search_query.trim().to_ascii_lowercase();
    let mut picked = None;

    if !q.is_empty() {
        for item in SYSML_V2_CATALOG
            .iter()
            .filter(|i| is_allowed_in_context(i, ctx))
        {
            if item.label.to_ascii_lowercase().contains(&q)
                || item.keyword.to_ascii_lowercase().contains(&q)
            {
                if catalog_item_row(ui, item, false, false).clicked() {
                    picked = Some(item);
                    ui.close();
                }
            }
        }
        return picked;
    }

    if matches!(ctx, CreateContext::InsideNode(_)) {
        ui.set_min_width(220.0);
        for item in SYSML_V2_CATALOG
            .iter()
            .filter(|i| is_allowed_in_context(i, ctx))
        {
            if catalog_item_row(ui, item, false, false).clicked() {
                picked = Some(item);
                ui.close();
            }
        }
        return picked;
    }

    for cat in CatalogCategory::ALL_CREATE {
        let items: Vec<&'static SysmlCatalogItem> = SYSML_V2_CATALOG
            .iter()
            .filter(|i| i.category == cat && is_allowed_in_context(i, ctx))
            .collect();
        if items.is_empty() {
            continue;
        }
        ui.menu_button(cat.label(), |ui| {
            ui.set_min_width(220.0);
            for item in items {
                if catalog_item_row(ui, item, false, false).clicked() {
                    picked = Some(item);
                    ui.close();
                }
            }
        });
    }
    picked
}

/// Optional collapsible SysML v2 Library Palette (closed by default; toggleable from diagram toolbar).
pub fn show_optional_palette(
    ui: &mut Ui,
    search: &mut String,
    active_tool: &mut Option<&'static SysmlCatalogItem>,
) {
    ui.set_width(184.0);
    ui.vertical(|ui| {
        if ui
            .selectable_label(active_tool.is_none(), "Select / Move")
            .clicked()
        {
            *active_tool = None;
        }
        ui.add(
            TextEdit::singleline(search)
                .desired_width(172.0)
                .hint_text("Filter SysML v2..."),
        );
        ui.separator();

        let q = search.trim().to_ascii_lowercase();
        ScrollArea::vertical()
            .id_salt("optional_sysml_palette_scroll")
            .show(ui, |ui| {
                for cat in CatalogCategory::ALL_CREATE
                    .into_iter()
                    .chain(std::iter::once(CatalogCategory::Relationships))
                {
                    let items: Vec<&'static SysmlCatalogItem> = SYSML_V2_CATALOG
                        .iter()
                        .filter(|i| {
                            i.category == cat && is_allowed_in_context(i, CreateContext::DiagramPalette)
                        })
                        .filter(|i| {
                            q.is_empty()
                                || i.label.to_ascii_lowercase().contains(&q)
                                || i.keyword.to_ascii_lowercase().contains(&q)
                        })
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    egui::CollapsingHeader::new(RichText::new(cat.label()).small().strong())
                        .open((!q.is_empty()).then_some(true))
                        .show(ui, |ui| {
                            for item in items {
                                let selected = active_tool.is_some_and(|t| t.id == item.id);
                                if catalog_item_row(ui, item, selected, true).clicked() {
                                    *active_tool = if selected { None } else { Some(item) };
                                }
                            }
                        });
                }
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_item(id: &str) -> &'static SysmlCatalogItem {
        SYSML_V2_CATALOG
            .iter()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("missing catalog item {id}"))
    }

    #[test]
    fn attribute_usage_excluded_from_palette_and_canvas_allowed_inside_part() {
        let attr = find_item("attr");
        assert!(!is_allowed_in_context(attr, CreateContext::DiagramPalette));
        assert!(!is_allowed_in_context(attr, CreateContext::EmptyCanvas));
        assert!(is_allowed_in_context(
            attr,
            CreateContext::InsideNode(NodeShape::PartBlock)
        ));
        assert!(is_allowed_in_context(attr, CreateContext::PackageExplorer));
    }

    #[test]
    fn port_usages_excluded_from_palette_and_canvas_allowed_inside_part() {
        for id in ["port", "conj_port"] {
            let item = find_item(id);
            assert!(!is_allowed_in_context(item, CreateContext::DiagramPalette));
            assert!(!is_allowed_in_context(item, CreateContext::EmptyCanvas));
            assert!(is_allowed_in_context(
                item,
                CreateContext::InsideNode(NodeShape::PartBlock)
            ));
        }
    }

    #[test]
    fn control_nodes_have_no_create_inside_items() {
        for shape in [
            NodeShape::StartCircle,
            NodeShape::DoneBullseye,
            NodeShape::ForkJoinBar,
            NodeShape::DecideDiamond,
            NodeShape::MergeDiamond,
        ] {
            assert!(!has_items_for_context(CreateContext::InsideNode(shape)));
        }
    }

    #[test]
    fn relationships_only_in_palette_not_empty_canvas_or_explorer() {
        for rel in catalog_relationships() {
            assert!(is_allowed_in_context(rel, CreateContext::DiagramPalette));
            assert!(!is_allowed_in_context(rel, CreateContext::EmptyCanvas));
            assert!(!is_allowed_in_context(rel, CreateContext::PackageExplorer));
            assert!(!is_allowed_in_context(
                rel,
                CreateContext::InsideNode(NodeShape::PartBlock)
            ));
        }
    }

    #[test]
    fn non_graphical_declarations_only_in_package_explorer() {
        for id in [
            "lib_pkg",
            "import",
            "alias",
            "filter",
            "attr_def",
            "general_view",
            "viewpoint_def",
            "rendering_def",
            "metadata_def",
        ] {
            let item = find_item(id);
            assert!(
                !is_allowed_in_context(item, CreateContext::DiagramPalette),
                "{id} must not appear in palette"
            );
            assert!(
                !is_allowed_in_context(item, CreateContext::EmptyCanvas),
                "{id} must not appear on empty canvas"
            );
            assert!(
                is_allowed_in_context(item, CreateContext::PackageExplorer),
                "{id} must remain available in Model Explorer"
            );
        }
    }
}
