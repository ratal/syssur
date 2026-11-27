//! Adapter over `Open-MBEE/sysml-toolkit` (`sysmlv2-syntax` + `sysmlv2-model`),
//! providing cross-file semantic validation, expression/constraint evaluation,
//! AST extraction into Syssur's `DefinitionGraph`, and canonical formatting.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};
use syssur_model::{
    ActionNode, ActionNodeKind, AttributeUsage, ConnectionEdge, ConstraintVerdictItem,
    DefinitionGraph, DiagnosticItem, DiagnosticSeverity, ElementId, ElementModifiers,
    FeatureOrigin, PackageNode, PartDef, PartUsage, PortDef, PortDirection, PortUsage,
    RelationshipKind, RequirementNode, Span, StateNode, SuccessionEdge, SysmlElement,
    SysmlElementKind, TraceLink, TransitionEdge, VerificationStatus, ViewDef, ViewKind,
};
use sysmlv2_model::check::{
    constraint_checks, satisfaction_checks, validate_model_with, validate_semantics_with,
    ConstraintVerdict,
};
use sysmlv2_model::json::ResolvedModel;
use sysmlv2_model::model::Model;
use sysmlv2_model::prepared::PreparedLibrary;
use sysmlv2_syntax::ast::{
    BinaryOp, DefKind, Definition, Expr, ExprKind, FeatureDirection, FeatureSpecialization,
    Literal, Member, MemberKind, Multiplicity, PortionKind, SourceUnit, TargetRef, TriggerKind,
    UnaryOp, Usage, UsageDetail, UsageKind, Visibility,
};
use sysmlv2_syntax::diag::Severity;
use sysmlv2_syntax::parser::parse_source;
use sysmlv2_syntax::print::print_source;

include!(concat!(env!("OUT_DIR"), "/stdlib_sources.rs"));

#[cfg(not(target_arch = "wasm32"))]
fn stdlib_content_key() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for (name, src) in STDLIB_SOURCES {
        feed(name.as_bytes());
        feed(&[0]);
        feed(src.as_bytes());
        feed(&[0]);
    }
    sysmlv2_model::ambient::mix_key(h)
}

fn build_prepared_standard_library() -> Arc<PreparedLibrary> {
    #[cfg(not(target_arch = "wasm32"))]
    let key = stdlib_content_key();

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(cache_dir) = sysmlv2_model::libcache::default_cache_dir() {
        let cache_path = cache_dir.join(format!("syssur-stdlib-{key:016x}.prepared"));
        if let Some(cached) = PreparedLibrary::load_cache(&cache_path, key) {
            return Arc::new(cached);
        }
    }

    let build_fn = move || {
        let mut lib_model = Model::new();
        for (name, src) in STDLIB_SOURCES {
            lib_model.add_library_source(*name, src);
        }
        sysmlv2_model::ambient::add_to(&mut lib_model);
        let prepared = PreparedLibrary::build(&lib_model).expect("prepare OMG SysML v2 stdlib");

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(cache_dir) = sysmlv2_model::libcache::default_cache_dir() {
            let _ = std::fs::create_dir_all(&cache_dir);
            let cache_path = cache_dir.join(format!("syssur-stdlib-{key:016x}.prepared"));
            let _ = prepared.save(&cache_path, key);
        }

        Arc::new(prepared)
    };

    #[cfg(not(target_arch = "wasm32"))]
    {
        // Build on an explicit 16 MiB stack so debug test threads never overflow.
        std::thread::Builder::new()
            .name("syssur-stdlib-prep".to_string())
            .stack_size(16 * 1024 * 1024)
            .spawn(build_fn)
            .map(|h| h.join().expect("stdlib prep thread panicked"))
            .unwrap_or_else(|_| build_fn())
    }
    #[cfg(target_arch = "wasm32")]
    {
        build_fn()
    }
}

fn prepared_standard_library() -> &'static Arc<PreparedLibrary> {
    static PREPARED: OnceLock<Arc<PreparedLibrary>> = OnceLock::new();
    PREPARED.get_or_init(build_prepared_standard_library)
}

#[derive(Clone, Debug)]
pub struct WorkspaceParseResult {
    pub graph: DefinitionGraph,
    pub parsed_units: BTreeMap<String, SourceUnit>,
}

pub struct SysmlEngine;

impl SysmlEngine {
    /// Parses all `.sysml` files, runs cross-file semantic validation and constraint
    /// evaluation via `sysmlv2-model` against the full OMG SysML v2 standard library,
    /// and extracts a unified `DefinitionGraph`.
    pub fn parse_workspace(files: &BTreeMap<String, String>) -> WorkspaceParseResult {
        let mut graph = DefinitionGraph::default();
        let mut parsed_units = BTreeMap::new();
        let mut model = Model::new();
        prepared_standard_library()
            .clone()
            .install(&mut model)
            .expect("install OMG SysML v2 stdlib");

        let mut unit_files: Vec<String> = vec![String::new(); model.unit_count()];
        let mut seen_diags: BTreeSet<(String, usize, usize, String)> = BTreeSet::new();

        for (file, content) in files {
            let parsed = parse_source(content);
            for diag in &parsed.diagnostics {
                let start = diag.span.start as usize;
                let end = diag.span.end as usize;
                if seen_diags.insert((file.clone(), start, end, diag.message.clone())) {
                    graph.diagnostics.push(DiagnosticItem {
                        severity: map_severity(diag.severity),
                        message: diag.message.clone(),
                        span: Span::new(file, start, end),
                    });
                }
            }
            extract_members(file, None, None, &parsed.unit.members, &mut graph);
            parsed_units.insert(file.clone(), parsed.unit);

            model.add_source(file.clone(), content);
            unit_files.push(file.clone());
        }

        // Cross-file referential & semantic validation + constraint checks in one resolve pass.
        let mut resolved = ResolvedModel::build(&model);
        for (unit_idx, diag) in validate_model_with(&mut resolved, &model)
            .into_iter()
            .chain(validate_semantics_with(&mut resolved, &model))
        {
            if model.is_library_unit(unit_idx) {
                continue;
            }
            let file = unit_files
                .get(unit_idx)
                .cloned()
                .unwrap_or_else(|| "<workspace>".to_string());
            let start = diag.span.start as usize;
            let end = diag.span.end as usize;
            if seen_diags.insert((file.clone(), start, end, diag.message.clone())) {
                graph.diagnostics.push(DiagnosticItem {
                    severity: map_severity(diag.severity),
                    message: diag.message,
                    span: Span::new(file, start, end),
                });
            }
        }

        // Evaluate requirement & constraint verdicts via sysmlv2-model
        for check in constraint_checks(&model, &mut resolved)
            .into_iter()
            .chain(satisfaction_checks(&model, &mut resolved))
        {
            if model.is_library_unit(check.unit) {
                continue;
            }
            let file = unit_files
                .get(check.unit)
                .cloned()
                .unwrap_or_else(|| "<workspace>".to_string());
            let status = match check.verdict {
                ConstraintVerdict::Satisfied => VerificationStatus::Satisfied,
                ConstraintVerdict::Violated => VerificationStatus::Violated,
                ConstraintVerdict::Undecided(reason) => VerificationStatus::Undecided(reason),
            };
            let name = check
                .name
                .unwrap_or_else(|| check.element_type.to_string());
            graph.verdicts.push(ConstraintVerdictItem {
                name,
                context: check.context,
                status,
                span: Span::new(file, check.span.start as usize, check.span.end as usize),
            });
        }

        WorkspaceParseResult {
            graph,
            parsed_units,
        }
    }

    /// Formats a `.sysml` document using `sysmlv2-syntax`'s canonical printer.
    pub fn format_source(content: &str) -> String {
        let parsed = parse_source(content);
        if parsed.has_errors() {
            return content.to_string();
        }
        print_source(&parsed.unit)
    }
}

fn map_severity(sev: Severity) -> DiagnosticSeverity {
    match sev {
        Severity::Error => DiagnosticSeverity::Error,
        Severity::Warning => DiagnosticSeverity::Warning,
    }
}

fn extract_members(
    file: &str,
    scope: Option<&ElementId>,
    enclosing_part_def: Option<&ElementId>,
    members: &[Member],
    graph: &mut DefinitionGraph,
) {
    for member in members {
        let span = Span::new(file, member.span.start as usize, member.span.end as usize);
        match &member.kind {
            MemberKind::Package(pkg) => {
                let name = pkg
                    .id
                    .name
                    .as_ref()
                    .or(pkg.id.short_name.as_ref())
                    .map(|n| n.value.clone())
                    .unwrap_or_else(|| "<anonymous>".to_string());
                let pkg_id = match scope {
                    Some(parent) => parent.child(&name),
                    None => ElementId::new(&name),
                };

                let mut imports = Vec::new();
                if let Some(body) = &pkg.body {
                    for child in body {
                        if let MemberKind::Import(imp) = &child.kind {
                            let mut target = imp.target.to_display_string();
                            if imp.is_recursive {
                                target.push_str("::**");
                            } else if imp.is_namespace {
                                target.push_str("::*");
                            }
                            imports.push(target);
                        }
                    }
                    extract_members(file, Some(&pkg_id), None, body, graph);
                }

                graph.packages.insert(
                    pkg_id.clone(),
                    PackageNode {
                        id: pkg_id,
                        name,
                        imports,
                        span,
                    },
                );
            }
            MemberKind::Definition(def) => {
                extract_definition(file, scope, member.visibility, def, span, graph);
            }
            MemberKind::Usage(usage) => {
                if let Some(parent_def_id) = enclosing_part_def {
                    attach_usage_to_part_def(file, scope, parent_def_id, usage, span, graph);
                } else {
                    extract_top_level_usage(file, scope, member.visibility, usage, span, graph);
                }
            }
            MemberKind::Dependency(dep) => {
                let owner = scope.cloned().unwrap_or_else(|| ElementId::new(""));
                let name = dep
                    .id
                    .name
                    .as_ref()
                    .or(dep.id.short_name.as_ref())
                    .map(|n| n.value.clone());
                for client in &dep.clients {
                    for supplier in &dep.suppliers {
                        let id = owner.child(name.as_deref().unwrap_or("dep"));
                        graph.connections.push(ConnectionEdge {
                            id,
                            name: name.clone(),
                            kind: RelationshipKind::Dependency,
                            interface_def: None,
                            owner_scope: owner.clone(),
                            source_path: client.to_display_string(),
                            target_path: supplier.to_display_string(),
                            span: span.clone(),
                        });
                    }
                }
            }
            MemberKind::Comment(c) => {
                let name = c
                    .id
                    .name
                    .as_ref()
                    .or(c.id.short_name.as_ref())
                    .map(|n| n.value.clone())
                    .unwrap_or_else(|| format!("comment_{}", span.start));
                let id = scope
                    .map(|p| p.child(&name))
                    .unwrap_or_else(|| ElementId::new(&name));
                graph.generic_elements.insert(
                    id.clone(),
                    SysmlElement {
                        id,
                        name,
                        kind: SysmlElementKind::Comment,
                        is_def: false,
                        short_id: c.id.short_name.as_ref().map(|n| n.value.clone()),
                        type_ref: None,
                        specializes: vec![],
                        doc: Some(c.body.trim().to_string()),
                        span,
                    },
                );
            }
            MemberKind::Alias(a) => {
                let name = a
                    .id
                    .name
                    .as_ref()
                    .or(a.id.short_name.as_ref())
                    .map(|n| n.value.clone())
                    .unwrap_or_else(|| a.target.to_display_string());
                let id = scope
                    .map(|p| p.child(&name))
                    .unwrap_or_else(|| ElementId::new(&name));
                graph.generic_elements.insert(
                    id.clone(),
                    SysmlElement {
                        id,
                        name,
                        kind: SysmlElementKind::Alias,
                        is_def: false,
                        short_id: a.id.short_name.as_ref().map(|n| n.value.clone()),
                        type_ref: Some(ElementId::new(a.target.to_display_string())),
                        specializes: vec![],
                        doc: None,
                        span,
                    },
                );
            }
            MemberKind::Filter(expr) => {
                let id = scope
                    .map(|p| p.child(&format!("filter_{}", span.start)))
                    .unwrap_or_else(|| ElementId::new(format!("filter_{}", span.start)));
                graph.generic_elements.insert(
                    id.clone(),
                    SysmlElement {
                        id,
                        name: format!("filter {}", format_expr(expr)),
                        kind: SysmlElementKind::Filter,
                        is_def: false,
                        short_id: None,
                        type_ref: None,
                        specializes: vec![],
                        doc: Some(format_expr(expr)),
                        span,
                    },
                );
            }
            MemberKind::TextualRep(rep) => {
                let name = rep
                    .id
                    .name
                    .as_ref()
                    .or(rep.id.short_name.as_ref())
                    .map(|n| n.value.clone())
                    .unwrap_or_else(|| format!("rep_{}", rep.language));
                let id = scope
                    .map(|p| p.child(&name))
                    .unwrap_or_else(|| ElementId::new(&name));
                graph.generic_elements.insert(
                    id.clone(),
                    SysmlElement {
                        id,
                        name,
                        kind: SysmlElementKind::TextualRep,
                        is_def: false,
                        short_id: rep.id.short_name.as_ref().map(|n| n.value.clone()),
                        type_ref: None,
                        specializes: vec![],
                        doc: Some(rep.body.trim().to_string()),
                        span,
                    },
                );
            }
            MemberKind::RequirementVerification(u) => {
                extract_verify_usage(scope, u, span, graph);
            }
            _ => {}
        }
    }
}

fn map_visibility(vis: Option<Visibility>) -> Option<String> {
    vis.map(|v| match v {
        Visibility::Public => "public".to_string(),
        Visibility::Private => "private".to_string(),
        Visibility::Protected => "protected".to_string(),
    })
}

fn def_modifiers(vis: Option<Visibility>, def: &Definition) -> ElementModifiers {
    ElementModifiers {
        visibility: map_visibility(vis),
        direction: None,
        is_abstract: def.prefix.is_abstract,
        is_variation: def.prefix.is_variation,
        is_variant: false,
        is_individual: def.prefix.is_individual,
        is_ref: false,
        is_derived: false,
        is_constant: false,
        is_parallel: def.is_parallel,
        value_expr: None,
    }
}

fn usage_modifiers(vis: Option<Visibility>, usage: &Usage) -> ElementModifiers {
    ElementModifiers {
        visibility: map_visibility(vis),
        direction: usage.prefix.direction.map(map_direction),
        is_abstract: usage.prefix.is_abstract,
        is_variation: usage.prefix.is_variation,
        is_variant: usage.prefix.is_variant,
        is_individual: usage.prefix.is_individual,
        is_ref: usage.prefix.is_ref,
        is_derived: usage.prefix.is_derived,
        is_constant: usage.prefix.is_constant,
        is_parallel: usage.is_parallel,
        value_expr: usage.value.as_ref().map(|v| format_expr(&v.expr)),
    }
}

fn extract_definition(
    file: &str,
    scope: Option<&ElementId>,
    vis: Option<Visibility>,
    def: &Definition,
    span: Span,
    graph: &mut DefinitionGraph,
) {
    let Some(name_node) = def.id.name.as_ref().or(def.id.short_name.as_ref()) else {
        return;
    };
    let name = name_node.value.clone();
    let def_id = match scope {
        Some(parent) => parent.child(&name),
        None => ElementId::new(&name),
    };
    graph
        .element_modifiers
        .insert(def_id.clone(), def_modifiers(vis, def));

    match def.kind {
        DefKind::Part => {
            let specializes = def
                .specializes
                .iter()
                .filter_map(target_ref_to_element_id)
                .collect();
            let mut part_def = PartDef {
                id: def_id.clone(),
                name,
                specializes,
                ports: Vec::new(),
                parts: Vec::new(),
                attributes: Vec::new(),
                attribute_usages: Vec::new(),
                span,
            };

            if let Some(body) = &def.body {
                for member in body {
                    let mspan = Span::new(file, member.span.start as usize, member.span.end as usize);
                    match &member.kind {
                        MemberKind::Usage(usage) => match usage.kind {
                            UsageKind::Part => {
                                if let Some(pu) = build_part_usage(file, &def_id, usage, mspan) {
                                    collect_nested_part_satisfies(
                                        file,
                                        &pu.id,
                                        usage.body.as_deref(),
                                        graph,
                                    );
                                    part_def.parts.push(pu);
                                }
                            }
                            UsageKind::Port => {
                                if let Some(port) = build_port_usage(&def_id, usage, mspan, graph) {
                                    part_def.ports.push(port);
                                }
                            }
                            UsageKind::Attribute => {
                                if let Some(attr) = build_attribute_usage(&def_id, usage, mspan) {
                                    part_def.attributes.push(attr.display_label());
                                    part_def.attribute_usages.push(attr);
                                }
                            }
                            UsageKind::Connection
                            | UsageKind::Interface
                            | UsageKind::Flow
                            | UsageKind::Allocation
                            | UsageKind::Binding => {
                                if let Some(edge) = build_connection_edge(&def_id, usage, mspan) {
                                    graph.connections.push(edge);
                                }
                            }
                            UsageKind::Satisfy => {
                                extract_satisfy_usage(&def_id, usage, mspan, graph);
                            }
                            UsageKind::Exhibit | UsageKind::State => {
                                if usage.body.as_ref().is_some_and(|b| !b.is_empty()) {
                                    if let Some(sname) = usage_name(usage) {
                                        let sid = def_id.child(&sname);
                                        let doc = extract_doc_from_members(usage.body.as_deref());
                                        let children = extract_state_body(
                                            file,
                                            &sid,
                                            usage.body.as_deref(),
                                            graph,
                                        );
                                        graph.states.insert(
                                            sid.clone(),
                                            StateNode {
                                                id: sid,
                                                name: sname,
                                                is_def: false,
                                                doc,
                                                children,
                                                span: mspan,
                                            },
                                        );
                                    }
                                }
                            }
                            UsageKind::Perform | UsageKind::Action => {
                                if usage.body.as_ref().is_some_and(|b| !b.is_empty()) {
                                    if let Some(aname) = usage_name(usage) {
                                        let aid = def_id.child(&aname);
                                        let doc = extract_doc_from_members(usage.body.as_deref());
                                        let children = extract_action_body(
                                            file,
                                            &aid,
                                            usage.body.as_deref(),
                                            graph,
                                        );
                                        graph.actions.insert(
                                            aid.clone(),
                                            ActionNode {
                                                id: aid,
                                                name: aname,
                                                kind: ActionNodeKind::Action,
                                                doc,
                                                children,
                                                span: mspan,
                                            },
                                        );
                                    }
                                }
                            }
                            _ => {}
                        },
                        MemberKind::RequirementVerification(u) => {
                            extract_verify_usage(Some(&def_id), u, mspan, graph);
                        }
                        _ => {}
                    }
                }
            }
            graph.part_defs.insert(def_id, part_def);
        }
        DefKind::Port => {
            let mut direction = PortDirection::InOut;
            let mut item_type = None;
            if let Some(body) = &def.body {
                for member in body {
                    if let MemberKind::Usage(usage) = &member.kind {
                        if let Some(dir) = usage.prefix.direction {
                            direction = map_direction(dir);
                        }
                        item_type = first_type_ref(usage).map(|(id, _)| id.0);
                    }
                }
            }
            graph.port_defs.insert(
                def_id.clone(),
                PortDef {
                    id: def_id,
                    name,
                    direction,
                    item_type,
                    span,
                },
            );
        }
        DefKind::Requirement => {
            let short_id = def.id.short_name.as_ref().map(|n| n.value.clone());
            let doc = extract_doc_from_members(def.body.as_deref());
            graph.requirements.insert(
                def_id.clone(),
                RequirementNode {
                    id: def_id,
                    short_id,
                    name,
                    doc,
                    is_usage: false,
                    span,
                },
            );
        }
        DefKind::Action => {
            let doc = extract_doc_from_members(def.body.as_deref());
            let children = extract_action_body(file, &def_id, def.body.as_deref(), graph);
            graph.actions.insert(
                def_id.clone(),
                ActionNode {
                    id: def_id,
                    name,
                    kind: ActionNodeKind::Action,
                    doc,
                    children,
                    span,
                },
            );
        }
        DefKind::State => {
            let doc = extract_doc_from_members(def.body.as_deref());
            let children = extract_state_body(file, &def_id, def.body.as_deref(), graph);
            graph.states.insert(
                def_id.clone(),
                StateNode {
                    id: def_id,
                    name,
                    is_def: true,
                    doc,
                    children,
                    span,
                },
            );
        }
        DefKind::View => {
            let expose_patterns = extract_expose_patterns(def.body.as_deref());
            let kind = infer_view_kind(&name, None);
            graph.views.insert(
                def_id.clone(),
                ViewDef {
                    id: def_id,
                    name,
                    kind,
                    is_usage: false,
                    expose_patterns,
                    include_relationships: Vec::new(),
                    span,
                },
            );
        }
        other_kind => {
            if let Some(elem_kind) = map_def_kind(other_kind) {
                let short_id = def.id.short_name.as_ref().map(|n| n.value.clone());
                let specializes = def
                    .specializes
                    .iter()
                    .filter_map(target_ref_to_element_id)
                    .collect();
                let doc = extract_doc_from_members(def.body.as_deref());
                if let Some(body) = &def.body {
                    extract_body_verifications(file, &def_id, body, graph);
                }
                graph.generic_elements.insert(
                    def_id.clone(),
                    SysmlElement {
                        id: def_id,
                        name,
                        kind: elem_kind,
                        is_def: true,
                        short_id,
                        type_ref: None,
                        specializes,
                        doc,
                        span,
                    },
                );
            }
        }
    }
}

fn attach_usage_to_part_def(
    file: &str,
    _scope: Option<&ElementId>,
    parent_def_id: &ElementId,
    usage: &Usage,
    span: Span,
    graph: &mut DefinitionGraph,
) {
    if let Some(part_def) = graph.part_defs.get_mut(parent_def_id) {
        if usage.kind == UsageKind::Part {
            if let Some(pu) = build_part_usage(file, parent_def_id, usage, span) {
                part_def.parts.push(pu);
            }
        }
    }
}

fn extract_top_level_usage(
    file: &str,
    scope: Option<&ElementId>,
    vis: Option<Visibility>,
    usage: &Usage,
    span: Span,
    graph: &mut DefinitionGraph,
) {
    let owner = scope.cloned().unwrap_or_else(|| ElementId::new(""));
    if let Some(name) = usage_name(usage) {
        let uid = if owner.as_str().is_empty() {
            ElementId::new(&name)
        } else {
            owner.child(&name)
        };
        graph
            .element_modifiers
            .insert(uid, usage_modifiers(vis, usage));
    }

    match usage.kind {
        UsageKind::Part => {
            if let Some(pu) = build_part_usage(file, &owner, usage, span) {
                collect_nested_part_satisfies(file, &pu.id, usage.body.as_deref(), graph);
                graph.top_level_parts.insert(pu.id.clone(), pu);
            }
        }
        UsageKind::Requirement => {
            if let Some(name) = usage_name(usage) {
                let req_id = if owner.as_str().is_empty() {
                    ElementId::new(&name)
                } else {
                    owner.child(&name)
                };
                let short_id = usage
                    .declaration
                    .id
                    .short_name
                    .as_ref()
                    .map(|n| n.value.clone());
                let doc = extract_doc_from_members(usage.body.as_deref());
                graph.requirements.insert(
                    req_id.clone(),
                    RequirementNode {
                        id: req_id,
                        short_id,
                        name,
                        doc,
                        is_usage: true,
                        span,
                    },
                );
            }
        }
        UsageKind::Action => {
            if let Some(name) = usage_name(usage) {
                let act_id = if owner.as_str().is_empty() {
                    ElementId::new(&name)
                } else {
                    owner.child(&name)
                };
                let doc = extract_doc_from_members(usage.body.as_deref());
                let children = extract_action_body(file, &act_id, usage.body.as_deref(), graph);
                graph.actions.insert(
                    act_id.clone(),
                    ActionNode {
                        id: act_id,
                        name,
                        kind: ActionNodeKind::Action,
                        doc,
                        children,
                        span,
                    },
                );
            }
        }
        UsageKind::State => {
            if let Some(name) = usage_name(usage) {
                let state_id = if owner.as_str().is_empty() {
                    ElementId::new(&name)
                } else {
                    owner.child(&name)
                };
                let doc = extract_doc_from_members(usage.body.as_deref());
                let children = extract_state_body(file, &state_id, usage.body.as_deref(), graph);
                graph.states.insert(
                    state_id.clone(),
                    StateNode {
                        id: state_id,
                        name,
                        is_def: false,
                        doc,
                        children,
                        span,
                    },
                );
            }
        }
        UsageKind::View => {
            if let Some(name) = usage_name(usage) {
                let view_id = if owner.as_str().is_empty() {
                    ElementId::new(&name)
                } else {
                    owner.child(&name)
                };
                let type_ref = first_type_ref(usage).map(|(id, _)| id);
                let kind = infer_view_kind(&name, type_ref.as_ref());
                let expose_patterns = extract_expose_patterns(usage.body.as_deref());
                graph.views.insert(
                    view_id.clone(),
                    ViewDef {
                        id: view_id,
                        name,
                        kind,
                        is_usage: true,
                        expose_patterns,
                        include_relationships: Vec::new(),
                        span,
                    },
                );
            }
        }
        UsageKind::Satisfy => {
            extract_satisfy_usage(&owner, usage, span, graph);
        }
        UsageKind::Connection
        | UsageKind::Interface
        | UsageKind::Flow
        | UsageKind::Allocation
        | UsageKind::Binding => {
            if let Some(edge) = build_connection_edge(&owner, usage, span.clone()) {
                graph.connections.push(edge);
            }
            if let Some(name) = usage_name(usage) {
                if let Some(elem_kind) = map_usage_kind(usage.kind, usage.prefix.portion) {
                    let elem_id = if owner.as_str().is_empty() {
                        ElementId::new(&name)
                    } else {
                        owner.child(&name)
                    };
                    graph.generic_elements.insert(
                        elem_id.clone(),
                        SysmlElement {
                            id: elem_id,
                            name,
                            kind: elem_kind,
                            is_def: false,
                            short_id: usage
                                .declaration
                                .id
                                .short_name
                                .as_ref()
                                .map(|n| n.value.clone()),
                            type_ref: first_type_ref(usage).map(|(t, _)| t),
                            specializes: vec![],
                            doc: extract_doc_from_members(usage.body.as_deref()),
                            span,
                        },
                    );
                }
            }
        }
        other_kind => {
            if let Some(elem_kind) = map_usage_kind(other_kind, usage.prefix.portion) {
                if let Some(name) = usage_name(usage) {
                    let elem_id = if owner.as_str().is_empty() {
                        ElementId::new(&name)
                    } else {
                        owner.child(&name)
                    };
                    let short_id = usage
                        .declaration
                        .id
                        .short_name
                        .as_ref()
                        .map(|n| n.value.clone());
                    let type_ref = first_type_ref(usage).map(|(t, _)| t);
                    let doc = extract_doc_from_members(usage.body.as_deref());
                    if let Some(body) = &usage.body {
                        extract_body_verifications(file, &elem_id, body, graph);
                    }
                    graph.generic_elements.insert(
                        elem_id.clone(),
                        SysmlElement {
                            id: elem_id,
                            name,
                            kind: elem_kind,
                            is_def: false,
                            short_id,
                            type_ref,
                            specializes: vec![],
                            doc,
                            span,
                        },
                    );
                }
            }
        }
    }
}

fn extract_body_verifications(
    file: &str,
    owner: &ElementId,
    body: &[Member],
    graph: &mut DefinitionGraph,
) {
    for m in body {
        let mspan = Span::new(file, m.span.start as usize, m.span.end as usize);
        match &m.kind {
            MemberKind::RequirementVerification(u) => {
                extract_verify_usage(Some(owner), u, mspan, graph);
            }
            MemberKind::Objective(obj) => {
                if let Some(obj_body) = &obj.body {
                    extract_body_verifications(file, owner, obj_body, graph);
                }
            }
            _ => {}
        }
    }
}

fn extract_verify_usage(
    owner: Option<&ElementId>,
    usage: &Usage,
    span: Span,
    graph: &mut DefinitionGraph,
) {
    let req = usage
        .declaration
        .specializations
        .iter()
        .find_map(|spec| match spec {
            FeatureSpecialization::References(t) => target_ref_to_element_id(t),
            FeatureSpecialization::Subsets(targets) => {
                targets.first().and_then(target_ref_to_element_id)
            }
            FeatureSpecialization::TypedBy(refs) => {
                refs.first().and_then(|tr| target_ref_to_element_id(&tr.target))
            }
            _ => None,
        })
        .or_else(|| usage_name(usage).map(ElementId::new));
    if let Some(req_id) = req {
        let verifier = owner
            .map(|o| o.short_name().to_string())
            .unwrap_or_else(|| "Verification".to_string());
        graph.trace_links.push(TraceLink {
            kind: RelationshipKind::Verify,
            requirement: req_id,
            target: verifier,
            span,
        });
    }
}

fn map_def_kind(kind: DefKind) -> Option<SysmlElementKind> {
    Some(match kind {
        DefKind::Attribute => SysmlElementKind::Attribute,
        DefKind::Enum => SysmlElementKind::Enum,
        DefKind::Item => SysmlElementKind::Item,
        DefKind::Occurrence | DefKind::Individual => SysmlElementKind::Occurrence,
        DefKind::Connection => SysmlElementKind::Connection,
        DefKind::Interface => SysmlElementKind::Interface,
        DefKind::Allocation => SysmlElementKind::Allocation,
        DefKind::Flow => SysmlElementKind::Flow,
        DefKind::Calc => SysmlElementKind::Calc,
        DefKind::Constraint => SysmlElementKind::Constraint,
        DefKind::Concern => SysmlElementKind::Concern,
        DefKind::Case => SysmlElementKind::Case,
        DefKind::Analysis => SysmlElementKind::AnalysisCase,
        DefKind::Verification => SysmlElementKind::VerificationCase,
        DefKind::UseCase => SysmlElementKind::UseCase,
        DefKind::Viewpoint => SysmlElementKind::Viewpoint,
        DefKind::Rendering => SysmlElementKind::Rendering,
        DefKind::Metadata => SysmlElementKind::Metadata,
        _ => return None,
    })
}

fn map_usage_kind(kind: UsageKind, portion: Option<PortionKind>) -> Option<SysmlElementKind> {
    if let Some(p) = portion {
        return Some(match p {
            PortionKind::Timeslice => SysmlElementKind::Timeslice,
            PortionKind::Snapshot => SysmlElementKind::Snapshot,
        });
    }
    Some(match kind {
        UsageKind::Attribute => SysmlElementKind::Attribute,
        UsageKind::Enum => SysmlElementKind::Enum,
        UsageKind::Item => SysmlElementKind::Item,
        UsageKind::Occurrence => SysmlElementKind::Occurrence,
        UsageKind::Connection => SysmlElementKind::Connection,
        UsageKind::Interface => SysmlElementKind::Interface,
        UsageKind::Allocation => SysmlElementKind::Allocation,
        UsageKind::Flow => SysmlElementKind::Flow,
        UsageKind::Calc => SysmlElementKind::Calc,
        UsageKind::Constraint => SysmlElementKind::Constraint,
        UsageKind::Concern => SysmlElementKind::Concern,
        UsageKind::Case => SysmlElementKind::Case,
        UsageKind::Analysis => SysmlElementKind::AnalysisCase,
        UsageKind::Verification => SysmlElementKind::VerificationCase,
        UsageKind::UseCase => SysmlElementKind::UseCase,
        UsageKind::Viewpoint => SysmlElementKind::Viewpoint,
        UsageKind::Rendering => SysmlElementKind::Rendering,
        UsageKind::Metadata => SysmlElementKind::Metadata,
        UsageKind::Perform => SysmlElementKind::PerformAction,
        UsageKind::Accept => SysmlElementKind::AcceptAction,
        UsageKind::Send => SysmlElementKind::SendAction,
        UsageKind::Assign => SysmlElementKind::AssignAction,
        UsageKind::Terminate => SysmlElementKind::TerminateAction,
        UsageKind::IfNode => SysmlElementKind::IfAction,
        UsageKind::WhileLoop | UsageKind::ForLoop => SysmlElementKind::LoopAction,
        UsageKind::Exhibit => SysmlElementKind::ExhibitState,
        UsageKind::Include => SysmlElementKind::IncludeUseCase,
        UsageKind::AssertConstraint => SysmlElementKind::AssertConstraint,
        UsageKind::Event => SysmlElementKind::EventOccurrence,
        UsageKind::Ref => SysmlElementKind::RefUsage,
        _ => return None,
    })
}

fn extract_state_body(
    file: &str,
    owner_state: &ElementId,
    body: Option<&[Member]>,
    graph: &mut DefinitionGraph,
) -> Vec<StateNode> {
    let Some(members) = body else {
        return Vec::new();
    };
    let mut children = Vec::new();
    let mut prev_state: Option<String> = None;

    for m in members {
        let mspan = Span::new(file, m.span.start as usize, m.span.end as usize);
        match &m.kind {
            MemberKind::InitialNode(qn) => {
                let target_name = qn.to_display_string();
                if target_name != "start" {
                    graph.transitions.push(TransitionEdge {
                        id: owner_state.child(&format!("trans_{}", graph.transitions.len())),
                        owner_scope: owner_state.clone(),
                        source: "start".to_string(),
                        target: target_name.clone(),
                        trigger: None,
                        guard: None,
                        effect: None,
                        span: mspan,
                    });
                }
                prev_state = Some(target_name);
            }
            MemberKind::Usage(u) => {
                if u.kind == UsageKind::State {
                    let name = usage_name(u).unwrap_or_else(|| "state".to_string());
                    let child_id = owner_state.child(&name);
                    let doc = extract_doc_from_members(u.body.as_deref());
                    let nested = extract_state_body(file, &child_id, u.body.as_deref(), graph);
                    children.push(StateNode {
                        id: child_id,
                        name: name.clone(),
                        is_def: false,
                        doc,
                        children: nested,
                        span: mspan,
                    });
                    prev_state = Some(name);
                } else if let UsageDetail::Transition {
                    source,
                    trigger,
                    guard,
                    effect,
                    target,
                    ..
                } = &u.detail
                {
                    let src_name = source
                        .as_ref()
                        .map(target_ref_display)
                        .or_else(|| prev_state.clone());
                    if let (Some(src), Some(tgt)) = (src_name, target) {
                        let dst_name = target_ref_display(&tgt.target);
                        let trigger_str = trigger.as_deref().and_then(format_trigger_detail);
                        let guard_str = guard.as_deref().map(format_expr);
                        let effect_str = effect.as_deref().and_then(format_effect_usage);
                        graph.transitions.push(TransitionEdge {
                            id: owner_state.child(&format!("trans_{}", graph.transitions.len())),
                            owner_scope: owner_state.clone(),
                            source: src,
                            target: dst_name.clone(),
                            trigger: trigger_str,
                            guard: guard_str,
                            effect: effect_str,
                            span: mspan,
                        });
                        prev_state = Some(dst_name);
                    }
                }
            }
            _ => {}
        }
    }
    children
}

fn format_trigger_detail(detail: &UsageDetail) -> Option<String> {
    let UsageDetail::Accept { payload, trigger, .. } = detail else {
        return None;
    };
    if let Some(trig) = trigger {
        let kw = match trig.kind {
            TriggerKind::At => "at",
            TriggerKind::After => "after",
            TriggerKind::When => "when",
        };
        return Some(format!("{kw} {}", format_expr(&trig.expr)));
    }
    let name = payload
        .id
        .name
        .as_ref()
        .or(payload.id.short_name.as_ref())
        .map(|n| n.value.clone());
    let ty = payload.specializations.iter().find_map(|s| match s {
        FeatureSpecialization::TypedBy(refs) => {
            refs.first().and_then(|r| target_ref_to_element_id(&r.target))
        }
        _ => None,
    });
    match (name, ty) {
        (Some(n), Some(t)) => Some(format!("{n} : {}", t.short_name())),
        (Some(n), None) => Some(n),
        (None, Some(t)) => Some(t.short_name().to_string()),
        (None, None) => None,
    }
}

fn format_effect_usage(u: &Usage) -> Option<String> {
    usage_name(u).or_else(|| first_type_ref(u).map(|(id, _)| id.short_name().to_string()))
}

fn collect_nested_part_satisfies(
    file: &str,
    part_id: &ElementId,
    body: Option<&[Member]>,
    graph: &mut DefinitionGraph,
) {
    let Some(members) = body else {
        return;
    };
    for m in members {
        let mspan = Span::new(file, m.span.start as usize, m.span.end as usize);
        if let MemberKind::Usage(u) = &m.kind {
            match u.kind {
                UsageKind::Satisfy => extract_satisfy_usage(part_id, u, mspan, graph),
                UsageKind::Connection
                | UsageKind::Interface
                | UsageKind::Flow
                | UsageKind::Allocation
                | UsageKind::Binding => {
                    if let Some(edge) = build_connection_edge(part_id, u, mspan) {
                        graph.connections.push(edge);
                    }
                }
                UsageKind::Part => {
                    if let Some(pname) = usage_name(u) {
                        collect_nested_part_satisfies(
                            file,
                            &part_id.child(&pname),
                            u.body.as_deref(),
                            graph,
                        );
                    }
                }
                UsageKind::Exhibit | UsageKind::State => {
                    if u.body.as_ref().is_some_and(|b| !b.is_empty()) {
                        if let Some(sname) = usage_name(u) {
                            let sid = part_id.child(&sname);
                            let doc = extract_doc_from_members(u.body.as_deref());
                            let children =
                                extract_state_body(file, &sid, u.body.as_deref(), graph);
                            graph.states.insert(
                                sid.clone(),
                                StateNode {
                                    id: sid,
                                    name: sname,
                                    is_def: false,
                                    doc,
                                    children,
                                    span: mspan,
                                },
                            );
                        }
                    }
                }
                UsageKind::Perform | UsageKind::Action => {
                    if u.body.as_ref().is_some_and(|b| !b.is_empty()) {
                        if let Some(aname) = usage_name(u) {
                            let aid = part_id.child(&aname);
                            let doc = extract_doc_from_members(u.body.as_deref());
                            let children =
                                extract_action_body(file, &aid, u.body.as_deref(), graph);
                            graph.actions.insert(
                                aid.clone(),
                                ActionNode {
                                    id: aid,
                                    name: aname,
                                    kind: ActionNodeKind::Action,
                                    doc,
                                    children,
                                    span: mspan,
                                },
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_satisfy_usage(
    owner: &ElementId,
    usage: &Usage,
    span: Span,
    graph: &mut DefinitionGraph,
) {
    let req = usage
        .declaration
        .specializations
        .iter()
        .find_map(|spec| match spec {
            FeatureSpecialization::References(t) => target_ref_to_element_id(t),
            FeatureSpecialization::Subsets(targets) => {
                targets.first().and_then(target_ref_to_element_id)
            }
            FeatureSpecialization::TypedBy(refs) => {
                refs.first().and_then(|tr| target_ref_to_element_id(&tr.target))
            }
            _ => None,
        })
        .or_else(|| usage_name(usage).map(ElementId::new));
    if let (Some(req_id), UsageDetail::Satisfy { by, .. }) = (req, &usage.detail) {
        let target = by
            .as_ref()
            .map(target_ref_display)
            .unwrap_or_else(|| owner.short_name().to_string());
        graph.trace_links.push(TraceLink {
            kind: RelationshipKind::Satisfy,
            requirement: req_id,
            target,
            span,
        });
    }
}

fn extract_doc_from_members(body: Option<&[Member]>) -> Option<String> {
    body.and_then(|members| {
        members.iter().find_map(|m| match &m.kind {
            MemberKind::Doc(d) => Some(d.body.trim().to_string()),
            _ => None,
        })
    })
}

fn extract_expose_patterns(body: Option<&[Member]>) -> Vec<String> {
    let mut expose_patterns = Vec::new();
    if let Some(members) = body {
        for m in members {
            if let MemberKind::Expose(imp) = &m.kind {
                let mut pat = imp.target.to_display_string();
                if imp.is_recursive {
                    pat.push_str("::**");
                } else if imp.is_namespace {
                    pat.push_str("::*");
                }
                expose_patterns.push(pat);
            }
        }
    }
    expose_patterns
}

fn infer_view_kind(name: &str, type_ref: Option<&ElementId>) -> ViewKind {
    if let Some(tr) = type_ref {
        let s = tr.short_name();
        if s.eq_ignore_ascii_case("StateTransitionView") || s.contains("StateTransition") {
            return ViewKind::StateTransition;
        }
        if s.eq_ignore_ascii_case("ActionFlowView") || s.contains("ActionFlow") {
            return ViewKind::ActionFlow;
        }
        if s.eq_ignore_ascii_case("InterconnectionView") || s.contains("Interconnection") {
            return ViewKind::Interconnection;
        }
        if s.eq_ignore_ascii_case("GeneralView") || s.contains("General") {
            return ViewKind::General;
        }
    }
    let lower = name.to_ascii_lowercase();
    if lower.contains("state") || lower.contains("transition") {
        ViewKind::StateTransition
    } else if lower.contains("action") || lower.contains("flow") || lower.contains("behavior") {
        ViewKind::ActionFlow
    } else if lower.contains("interconnection") || lower.contains("interface") {
        ViewKind::Interconnection
    } else {
        ViewKind::General
    }
}

fn extract_action_body(
    file: &str,
    owner_action: &ElementId,
    body: Option<&[Member]>,
    graph: &mut DefinitionGraph,
) -> Vec<ActionNode> {
    let Some(members) = body else {
        return Vec::new();
    };
    let mut children = Vec::new();
    let mut prev_node: Option<String> = None;
    let mut has_start = false;
    let mut has_done = false;
    let mut synthetic_start_span = Span::new(file, 0, 0);
    let mut synthetic_done_span = Span::new(file, 0, 0);

    for m in members {
        let mspan = Span::new(file, m.span.start as usize, m.span.end as usize);
        match &m.kind {
            MemberKind::InitialNode(qn) => {
                let target_name = qn.to_display_string();
                if target_name == "start" {
                    has_start = true;
                    synthetic_start_span = mspan.clone();
                    prev_node = Some("start".to_string());
                } else {
                    has_start = true;
                    synthetic_start_span = mspan.clone();
                    graph.successions.push(SuccessionEdge {
                        id: owner_action.child(&format!("succ_start_{}", graph.successions.len())),
                        owner_scope: owner_action.clone(),
                        source: "start".to_string(),
                        target: target_name.clone(),
                        guard: None,
                        span: mspan,
                    });
                    prev_node = Some(target_name);
                }
            }
            MemberKind::Usage(u) => {
                let kind_opt = match u.kind {
                    UsageKind::Action => Some(ActionNodeKind::Action),
                    UsageKind::Fork => Some(ActionNodeKind::Fork),
                    UsageKind::Join => Some(ActionNodeKind::Join),
                    UsageKind::Decide => Some(ActionNodeKind::Decide),
                    UsageKind::Merge => Some(ActionNodeKind::Merge),
                    _ => None,
                };
                if let Some(nk) = kind_opt {
                    let name = usage_name(u).unwrap_or_else(|| format!("{nk:?}"));
                    let child_id = owner_action.child(&name);
                    let doc = extract_doc_from_members(u.body.as_deref());
                    let nested = extract_action_body(file, &child_id, u.body.as_deref(), graph);
                    if m.leading_then {
                        if let Some(ref prev) = prev_node {
                            graph.successions.push(SuccessionEdge {
                                id: owner_action
                                    .child(&format!("succ_{}", graph.successions.len())),
                                owner_scope: owner_action.clone(),
                                source: prev.clone(),
                                target: name.clone(),
                                guard: None,
                                span: mspan.clone(),
                            });
                        }
                    }
                    children.push(ActionNode {
                        id: child_id,
                        name: name.clone(),
                        kind: nk,
                        doc,
                        children: nested,
                        span: mspan,
                    });
                    prev_node = Some(name);
                    continue;
                }

                match &u.detail {
                    UsageDetail::Succession { source, target } => {
                        let src_name = source
                            .as_ref()
                            .map(|s| target_ref_display(&s.target))
                            .or_else(|| prev_node.clone());
                        let dst_name = target_ref_display(&target.target);
                        if let Some(src) = src_name {
                            if src == "start" {
                                has_start = true;
                                synthetic_start_span = mspan.clone();
                            }
                            if dst_name == "done" {
                                has_done = true;
                                synthetic_done_span = mspan.clone();
                            }
                            graph.successions.push(SuccessionEdge {
                                id: owner_action
                                    .child(&format!("succ_{}", graph.successions.len())),
                                owner_scope: owner_action.clone(),
                                source: src,
                                target: dst_name.clone(),
                                guard: None,
                                span: mspan,
                            });
                            prev_node = Some(dst_name);
                        }
                    }
                    UsageDetail::Transition {
                        source,
                        guard,
                        target,
                        ..
                    } => {
                        let src_name = source
                            .as_ref()
                            .map(target_ref_display)
                            .or_else(|| prev_node.clone());
                        if let (Some(src), Some(tgt)) = (src_name, target) {
                            let dst_name = target_ref_display(&tgt.target);
                            if src == "start" {
                                has_start = true;
                                synthetic_start_span = mspan.clone();
                            }
                            if dst_name == "done" {
                                has_done = true;
                                synthetic_done_span = mspan.clone();
                            }
                            let guard_text = guard.as_deref().map(format_expr);
                            graph.successions.push(SuccessionEdge {
                                id: owner_action
                                    .child(&format!("succ_{}", graph.successions.len())),
                                owner_scope: owner_action.clone(),
                                source: src,
                                target: dst_name.clone(),
                                guard: guard_text,
                                span: mspan,
                            });
                            prev_node = Some(dst_name);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if has_start && !children.iter().any(|c| c.kind == ActionNodeKind::Start) {
        children.insert(
            0,
            ActionNode {
                id: owner_action.child("start"),
                name: "start".to_string(),
                kind: ActionNodeKind::Start,
                doc: None,
                children: Vec::new(),
                span: synthetic_start_span,
            },
        );
    }
    if has_done && !children.iter().any(|c| c.kind == ActionNodeKind::Done) {
        children.push(ActionNode {
            id: owner_action.child("done"),
            name: "done".to_string(),
            kind: ActionNodeKind::Done,
            doc: None,
            children: Vec::new(),
            span: synthetic_done_span,
        });
    }

    children
}

fn format_expr(expr: &Expr) -> String {
    match &expr.kind {
        ExprKind::Literal(lit) => match lit {
            Literal::Bool(b) => b.to_string(),
            Literal::String(s) => format!("\"{s}\""),
            Literal::Integer(n) | Literal::Real(n) => n.clone(),
            Literal::Infinity => "*".to_string(),
        },
        ExprKind::Null => "null".to_string(),
        ExprKind::Ref(qn) => qn.to_display_string(),
        ExprKind::Unary { op, operand } => {
            let prefix = match op {
                UnaryOp::Plus => "+",
                UnaryOp::Minus => "-",
                UnaryOp::Tilde => "~",
                UnaryOp::Not => "not ",
            };
            format!("{prefix}{}", format_expr(operand))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let sym = match op {
                BinaryOp::NullCoalescing => "??",
                BinaryOp::Implies => "implies",
                BinaryOp::OrBar => "|",
                BinaryOp::CondOr => "or",
                BinaryOp::Xor => "xor",
                BinaryOp::AndAmp => "&",
                BinaryOp::CondAnd => "and",
                BinaryOp::Eq => "==",
                BinaryOp::NotEq => "!=",
                BinaryOp::Same => "===",
                BinaryOp::NotSame => "!==",
                BinaryOp::Lt => "<",
                BinaryOp::Gt => ">",
                BinaryOp::LtEq => "<=",
                BinaryOp::GtEq => ">=",
                BinaryOp::Range => "..",
                BinaryOp::Add => "+",
                BinaryOp::Sub => "-",
                BinaryOp::Mul => "*",
                BinaryOp::Div => "/",
                BinaryOp::Rem => "%",
                BinaryOp::Pow => "**",
                BinaryOp::Caret => "^",
            };
            format!("{} {sym} {}", format_expr(lhs), format_expr(rhs))
        }
        ExprKind::ChainStep { target, member } => {
            format!("{}.{}", format_expr(target), target_ref_display(member))
        }
        ExprKind::Bracket { target, arg } => {
            format!("{} [{}]", format_expr(target), format_expr(arg))
        }
        ExprKind::Conditional {
            cond,
            then_branch,
            else_branch,
        } => format!(
            "if {} ? {} else {}",
            format_expr(cond),
            format_expr(then_branch),
            format_expr(else_branch)
        ),
        _ => "<expr>".to_string(),
    }
}

fn build_part_usage(
    file: &str,
    parent_id: &ElementId,
    usage: &Usage,
    span: Span,
) -> Option<PartUsage> {
    let origin = detect_feature_origin(usage);
    let name = usage_name(usage).or_else(|| match &origin {
        FeatureOrigin::Redefined(target) => Some(target.short_name().to_string()),
        FeatureOrigin::Subsetted(target) => Some(target.short_name().to_string()),
        _ => None,
    })?;

    let id = parent_id.child(&name);
    let type_def = first_type_ref(usage).map(|(t, _)| t);
    let multiplicity = usage
        .declaration
        .multiplicity
        .as_deref()
        .and_then(extract_multiplicity_count);

    let mut nested_parts = Vec::new();
    let mut ports = Vec::new();
    let mut attributes = Vec::new();
    if let Some(body) = &usage.body {
        for member in body {
            let mspan = Span::new(file, member.span.start as usize, member.span.end as usize);
            if let MemberKind::Usage(child) = &member.kind {
                if child.kind == UsageKind::Part {
                    if let Some(np) = build_part_usage(file, &id, child, mspan) {
                        nested_parts.push(np);
                    }
                } else if child.kind == UsageKind::Port {
                    let pd = first_type_ref(child);
                    let pname = usage_name(child).unwrap_or_else(|| "port".into());
                    ports.push(PortUsage {
                        id: id.child(&pname),
                        name: pname,
                        port_def: pd.as_ref().map(|(t, _)| t.clone()),
                        is_conjugated: pd.as_ref().is_some_and(|(_, conj)| *conj),
                        direction: child
                            .prefix
                            .direction
                            .map(map_direction)
                            .unwrap_or(PortDirection::InOut),
                        origin: detect_feature_origin(child),
                        span: mspan,
                    });
                } else if child.kind == UsageKind::Attribute {
                    if let Some(attr) = build_attribute_usage(&id, child, mspan) {
                        attributes.push(attr);
                    }
                }
            }
        }
    }

    Some(PartUsage {
        id,
        name,
        type_def,
        multiplicity,
        origin,
        ports,
        nested_parts,
        attributes,
        span,
    })
}

fn build_attribute_usage(
    parent_id: &ElementId,
    usage: &Usage,
    span: Span,
) -> Option<AttributeUsage> {
    let origin = detect_feature_origin(usage);
    let name = usage_name(usage).or_else(|| match &origin {
        FeatureOrigin::Redefined(target) => Some(target.short_name().to_string()),
        FeatureOrigin::Subsetted(target) => Some(target.short_name().to_string()),
        _ => None,
    })?;
    let type_name = first_type_ref(usage)
        .map(|(id, _)| id.0)
        .or_else(|| {
            usage.declaration.specializations.iter().find_map(|s| match s {
                FeatureSpecialization::Subsets(targets) => targets
                    .first()
                    .and_then(target_ref_to_element_id)
                    .map(|id| format!(":> {}", id.0)),
                _ => None,
            })
        });
    let value_expr = usage.value.as_ref().map(|v| format_expr(&v.expr));
    Some(AttributeUsage {
        id: parent_id.child(&name),
        name,
        type_name,
        value_expr,
        origin,
        span,
    })
}

fn build_port_usage(
    parent_id: &ElementId,
    usage: &Usage,
    span: Span,
    graph: &DefinitionGraph,
) -> Option<PortUsage> {
    let name = usage_name(usage)?;
    let type_info = first_type_ref(usage);
    let is_conjugated = type_info.as_ref().is_some_and(|(_, c)| *c);
    let port_def = type_info.map(|(id, _)| id);

    let direction = if let Some(explicit_dir) = usage.prefix.direction {
        map_direction(explicit_dir).effective_direction(is_conjugated)
    } else if let Some(ref pd_id) = port_def {
        if let Some(pd) = graph.resolve_port_def(pd_id) {
            pd.direction.effective_direction(is_conjugated)
        } else {
            PortDirection::InOut
        }
    } else {
        PortDirection::InOut
    };

    Some(PortUsage {
        id: parent_id.child(&name),
        name,
        port_def,
        is_conjugated,
        direction,
        origin: detect_feature_origin(usage),
        span,
    })
}

fn build_connection_edge(
    owner_scope: &ElementId,
    usage: &Usage,
    span: Span,
) -> Option<ConnectionEdge> {
    let name = usage_name(usage);
    let id = owner_scope.child(name.as_deref().unwrap_or("conn"));
    let interface_def = first_type_ref(usage).map(|(id, _)| id);

    match &usage.detail {
        UsageDetail::Connector { ends } if ends.len() >= 2 => {
            let kind = match usage.kind {
                UsageKind::Interface => RelationshipKind::Interface,
                UsageKind::Allocation => RelationshipKind::Allocate,
                _ => RelationshipKind::Connection,
            };
            Some(ConnectionEdge {
                id,
                name,
                kind,
                interface_def,
                owner_scope: owner_scope.clone(),
                source_path: target_ref_display(&ends[0].target),
                target_path: target_ref_display(&ends[1].target),
                span,
            })
        }
        UsageDetail::Binding { ends } if ends.len() >= 2 => Some(ConnectionEdge {
            id,
            name,
            kind: RelationshipKind::Binding,
            interface_def,
            owner_scope: owner_scope.clone(),
            source_path: target_ref_display(&ends[0].target),
            target_path: target_ref_display(&ends[1].target),
            span,
        }),
        UsageDetail::Flow {
            source: Some(src),
            target: Some(dst),
            payload,
        } => {
            let payload_lbl = payload.as_ref().and_then(|p| {
                p.specializations.iter().find_map(|s| match s {
                    FeatureSpecialization::TypedBy(refs) => refs
                        .first()
                        .and_then(|r| target_ref_to_element_id(&r.target))
                        .map(|t| t.short_name().to_string()),
                    _ => None,
                })
            });
            Some(ConnectionEdge {
                id,
                name: name.or(payload_lbl),
                kind: RelationshipKind::Flow,
                interface_def,
                owner_scope: owner_scope.clone(),
                source_path: target_ref_display(&src.target),
                target_path: target_ref_display(&dst.target),
                span,
            })
        }
        _ => None,
    }
}

fn detect_feature_origin(usage: &Usage) -> FeatureOrigin {
    for spec in &usage.declaration.specializations {
        match spec {
            FeatureSpecialization::Redefines(targets) => {
                if let Some(id) = targets.first().and_then(target_ref_to_element_id) {
                    return FeatureOrigin::Redefined(id);
                }
            }
            FeatureSpecialization::Subsets(targets) => {
                if let Some(id) = targets.first().and_then(target_ref_to_element_id) {
                    return FeatureOrigin::Subsetted(id);
                }
            }
            _ => {}
        }
    }
    FeatureOrigin::Owned
}

fn first_type_ref(usage: &Usage) -> Option<(ElementId, bool)> {
    for spec in &usage.declaration.specializations {
        if let FeatureSpecialization::TypedBy(refs) = spec {
            if let Some(tr) = refs.first() {
                if let Some(id) = target_ref_to_element_id(&tr.target) {
                    return Some((id, tr.is_conjugated));
                }
            }
        }
    }
    None
}

fn usage_name(usage: &Usage) -> Option<String> {
    usage
        .declaration
        .id
        .name
        .as_ref()
        .or(usage.declaration.id.short_name.as_ref())
        .map(|n| n.value.clone())
}

fn target_ref_to_element_id(target: &TargetRef) -> Option<ElementId> {
    if target.is_unspelled() {
        return None;
    }
    Some(ElementId::new(target_ref_display(target)))
}

fn target_ref_display(target: &TargetRef) -> String {
    match target {
        TargetRef::Name(qn) => qn.to_display_string(),
        TargetRef::Chain(links) => links
            .iter()
            .map(|qn| qn.to_display_string())
            .collect::<Vec<_>>()
            .join("."),
    }
}

fn extract_multiplicity_count(mult: &Multiplicity) -> Option<u32> {
    if let ExprKind::Literal(Literal::Integer(raw)) = &mult.upper.kind {
        return raw.parse::<u32>().ok();
    }
    None
}

fn map_direction(dir: FeatureDirection) -> PortDirection {
    match dir {
        FeatureDirection::In => PortDirection::In,
        FeatureDirection::Out => PortDirection::Out,
        FeatureDirection::InOut => PortDirection::InOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syssur_model::InstanceNode;

    #[test]
    fn parses_rover_system_example_cleanly() {
        let mut files = BTreeMap::new();
        files.insert(
            "power.sysml".into(),
            include_str!("../../../examples/rover-system/power.sysml").into(),
        );
        files.insert(
            "chassis.sysml".into(),
            include_str!("../../../examples/rover-system/chassis.sysml").into(),
        );
        files.insert(
            "vehicle.sysml".into(),
            include_str!("../../../examples/rover-system/vehicle.sysml").into(),
        );
        files.insert(
            "views.sysml".into(),
            include_str!("../../../examples/rover-system/views.sysml").into(),
        );

        let parsed = SysmlEngine::parse_workspace(&files);
        let errors: Vec<_> = parsed
            .graph
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "unexpected error diagnostics: {:?}",
            errors
        );
        assert!(parsed.graph.part_defs.contains_key(&ElementId::new("Power::Powertrain")));
        assert!(parsed.graph.part_defs.contains_key(&ElementId::new("RoverSystem::HeavyDutyRover")));
        assert!(parsed.graph.views.contains_key(&ElementId::new("RoverViews::StructuralBreakdown")));
        assert_eq!(parsed.graph.trace_links.len(), 2);
        assert!(
            parsed
                .graph
                .verdicts
                .iter()
                .any(|v| v.status == VerificationStatus::Satisfied),
            "expected at least one Satisfied requirement verdict, got: {:?}",
            parsed.graph.verdicts
        );

        let rover_tree = InstanceNode::expand_from_graph(
            &parsed.graph,
            &ElementId::new("RoverSystem::Rover"),
            4,
        )
        .expect("rover instance tree");
        assert_eq!(rover_tree.children.len(), 2);
    }

    #[test]
    fn evaluates_violated_requirement_and_reports_semantic_diagnostics() {
        let mut files = BTreeMap::new();
        files.insert(
            "power.sysml".into(),
            include_str!("../../../examples/rover-system/power.sysml")
                .replace("12.0", "8.0"),
        );
        files.insert(
            "chassis.sysml".into(),
            include_str!("../../../examples/rover-system/chassis.sysml").into(),
        );
        files.insert(
            "vehicle.sysml".into(),
            include_str!("../../../examples/rover-system/vehicle.sysml").into(),
        );
        let parsed = SysmlEngine::parse_workspace(&files);
        assert!(
            parsed
                .graph
                .verdicts
                .iter()
                .any(|v| v.status == VerificationStatus::Violated),
            "expected Violated verdict when maxPowerKW=8.0 < 10.0, got: {:?}",
            parsed.graph.verdicts
        );

        let mut broken = BTreeMap::new();
        broken.insert(
            "bad.sysml".into(),
            "package Bad { part x : NonExistentPartDef; }\n".into(),
        );
        let bad_parsed = SysmlEngine::parse_workspace(&broken);
        assert!(
            !bad_parsed.graph.diagnostics.is_empty(),
            "expected semantic diagnostic for unresolved type"
        );
    }

    #[test]
    fn parses_doorbell_system_example_cleanly() {
        let mut files = BTreeMap::new();
        files.insert(
            "structure.sysml".into(),
            include_str!("../../../examples/doorbell-system/structure.sysml").into(),
        );
        files.insert(
            "behavior.sysml".into(),
            include_str!("../../../examples/doorbell-system/behavior.sysml").into(),
        );
        files.insert(
            "requirements.sysml".into(),
            include_str!("../../../examples/doorbell-system/requirements.sysml").into(),
        );

        let parsed = SysmlEngine::parse_workspace(&files);
        let errors: Vec<_> = parsed
            .graph
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "unexpected error diagnostics in doorbell-system: {:?}",
            errors
        );

        let story = parsed
            .graph
            .resolve_action(&ElementId::new("System Behavior::Visitor Arrival Story"))
            .expect("Visitor Arrival Story action");
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Start));
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Fork));
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Join));
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Decide));
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Merge));
        assert!(story.children.iter().any(|c| c.kind == ActionNodeKind::Done));

        assert!(
            parsed
                .graph
                .successions
                .iter()
                .any(|s| s.guard.as_deref() == Some("timeElapsed == false")),
            "expected guarded succession, got: {:?}",
            parsed.graph.successions
        );

        let ch02 = parsed
            .graph
            .requirements
            .get(&ElementId::new("Requirements::Chime Cycle"))
            .expect("CH-02 requirement");
        assert_eq!(ch02.short_id.as_deref(), Some("CH-02"));
        assert!(ch02.doc.as_deref().unwrap_or_default().contains("three chime tones"));

        let arrival_view = parsed
            .graph
            .views
            .get(&ElementId::new("System Behavior::Visitor Arrival"))
            .expect("Visitor Arrival view");
        assert_eq!(arrival_view.kind, ViewKind::ActionFlow);
        assert_eq!(parsed.graph.trace_links.len(), 2);
    }

    #[test]
    fn parses_full_clause7_constructs_cleanly() {
        let mut files = BTreeMap::new();
        files.insert(
            "clause7.sysml".into(),
            r#"
package Clause7Demo {
    import ScalarValues::*;
    import StandardViewDefinitions::*;

    item def SignalPacket;
    attribute def MassKG;
    enum def ModeEnum { enum idle; enum active; }

    abstract part def BaseController {
        attribute mass : MassKG;
    }

    part controller : BaseController {
        part subA {
            out item sigOut : SignalPacket;
        }
        part subB {
            in item sigIn : SignalPacket;
        }
        flow of SignalPacket from subA.sigOut to subB.sigIn;
        bind subA = subB;
    }

    state def ControllerStates {
        first Off;
        state Off;
        state Active;
        transition first Off accept startCmd if true then Active;
    }

    requirement <'R-1'> SafetyReq {
        doc /* Must stay safe. */
    }
    constraint def SafeMargin {
        doc /* margin > 0 */
    }
    use case def OperateSystem;
    analysis def PowerAnalysis;
    verification def SafetyCheck {
        objective {
            verify SafetyReq;
        }
    }

    dependency from controller to BaseController;
    allocate controller.subA to controller.subB;
    comment note1 /* Clause 7 note */
    view stateView : StateTransitionView {
        expose Clause7Demo::**;
    }
}
"#
            .into(),
        );

        let parsed = SysmlEngine::parse_workspace(&files);
        let errors: Vec<_> = parsed
            .graph
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        assert!(parsed.graph.resolve_state(&ElementId::new("ControllerStates")).is_some());
        assert_eq!(parsed.graph.transitions.len(), 2);
        assert!(parsed
            .graph
            .connections
            .iter()
            .any(|c| c.kind == RelationshipKind::Flow));
        assert!(parsed
            .graph
            .connections
            .iter()
            .any(|c| c.kind == RelationshipKind::Allocate));
        assert!(parsed
            .graph
            .connections
            .iter()
            .any(|c| c.kind == RelationshipKind::Dependency));
        assert!(parsed
            .graph
            .trace_links
            .iter()
            .any(|l| l.kind == RelationshipKind::Verify));
        assert!(parsed
            .graph
            .element_modifiers
            .get(&ElementId::new("Clause7Demo::BaseController"))
            .is_some_and(|m| m.is_abstract));
        assert_eq!(
            parsed
                .graph
                .views
                .get(&ElementId::new("Clause7Demo::stateView"))
                .map(|v| v.kind),
            Some(ViewKind::StateTransition)
        );
    }

    #[test]
    fn parses_ptc_25_04_31_simple_vehicle_model_against_stdlib() {
        let src = include_str!("../../../docs/ptc-25-04-31.sysml");
        assert!(
            parse_source(src).diagnostics.is_empty(),
            "expected 0 syntax parse errors on ptc-25-04-31.sysml"
        );
        let mut files = BTreeMap::new();
        files.insert("ptc-25-04-31.sysml".into(), src.into());
        let parsed = SysmlEngine::parse_workspace(&files);
        assert!(
            parsed.graph.diagnostics.len() <= 30,
            "expected <= 30 genuine model diagnostics after stdlib load (down from 252), got {}",
            parsed.graph.diagnostics.len()
        );
        for pkg in [
            "ISQ",
            "SI",
            "Quantities",
            "Views",
            "ModelingMetadata",
            "ShapeItems",
            "TradeStudies",
            "VerificationCases",
            "RiskMetadata",
            "Time::DateTime",
        ] {
            let needle = format!("unresolved reference `{pkg}");
            assert!(
                !parsed
                    .graph
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains(&needle)),
                "unexpected unresolved stdlib reference to {pkg}"
            );
        }
        assert!(!parsed.graph.part_defs.is_empty());
        assert!(parsed.graph.top_level_parts.keys().any(|id| id.short_name() == "vehicle_b"));
        assert!(parsed.graph.states.keys().any(|id| id.short_name() == "vehicleStates"));
        assert!(parsed
            .graph
            .views
            .keys()
            .any(|id| id.short_name() == "vehiclePartsTree_Safety"));
        assert!(!parsed.graph.requirements.is_empty());
    }

    #[test]
    fn extracts_nested_part_connections_and_explicit_port_directions() {
        let mut files = BTreeMap::new();
        files.insert(
            "m.sysml".into(),
            r#"package RoverPkg {
    port def PowerPort { out item power; }
    part def Battery {
        out port supply : PowerPort;
    }
    part def Distribution {
        in port input : PowerPort;
    }
    part def InspectionRover {
        part energy {
            part battery : Battery;
            part distribution : Distribution;
            connection batterySupply connect battery.supply to distribution.input;
        }
    }
}"#
            .into(),
        );
        let parsed = SysmlEngine::parse_workspace(&files);
        assert!(
            parsed
                .graph
                .connections
                .iter()
                .any(|c| c.name.as_deref() == Some("batterySupply")
                    && c.source_path == "battery.supply"
                    && c.target_path == "distribution.input"),
            "expected nested batterySupply connection inside part energy, got {:?}",
            parsed.graph.connections
        );
        let dist = parsed
            .graph
            .resolve_part_def(&ElementId::new("RoverPkg::Distribution"))
            .expect("Distribution part def");
        let input_port = dist
            .ports
            .iter()
            .find(|p| p.name == "input")
            .expect("input port");
        assert_eq!(
            input_port.direction,
            PortDirection::In,
            "explicit `in port` must keep PortDirection::In"
        );
    }
}
