//! Right-side tabbed inspector panel (`[Details | .sysml Code]`).
//! Supports full OMG SysML v2 Clause 7 property editing, modifiers, visibility/direction,
//! Def<->Usage transformation, deletion, and live `.sysml` text editing.

use crate::i18n::{tr, Locale, TextKey};
use egui::{Color32, RichText, ScrollArea, TextEdit, Ui};
use std::collections::BTreeMap;
use syssur_cmd::SyssurCommand;
use syssur_collab::ProjectCollab;
use syssur_layout::{LayoutOutput, PortPlacement, PortSide};
use syssur_model::{
    ActionNode, ActionNodeKind, AttributeUsage, DefinitionGraph, DiagnosticSeverity, ElementId,
    ElementModifiers, FeatureOrigin, InstancePath, PortDirection, RelationshipKind, StateNode,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InspectorTab {
    #[default]
    Details,
    Code,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SelectedTarget {
    #[default]
    None,
    Node {
        path: InstancePath,
        element_id: ElementId,
    },
    Element(ElementId),
    Edge {
        source: String,
        target: String,
        kind: RelationshipKind,
        trigger: Option<String>,
        guard: Option<String>,
        effect: Option<String>,
        file: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct InspectorDraft {
    synced_key: String,
    pub name: String,
    pub short_id: String,
    pub doc: String,
    pub type_name: String,
    pub multiplicity: String,
    pub value_expr: String,
    pub specializes: String,
    pub attr_drafts: BTreeMap<String, (String, String, String)>,
    pub new_attr_name: String,
    pub new_attr_type: String,
    pub new_attr_value: String,
    pub new_port_name: String,
    pub new_port_type: String,
    pub new_port_conjugated: bool,
    pub trigger: String,
    pub guard: String,
    pub effect: String,
}

impl InspectorDraft {
    pub fn invalidate(&mut self) {
        self.synced_key.clear();
    }
}

pub enum InspectorAction {
    ApplyCommand(SyssurCommand),
    MovePort {
        node_key: String,
        port_name: String,
        placement: Option<PortPlacement>,
    },
    EditFileText { file: String, text: String },
    FormatFile { file: String },
    SelectFile(String),
    JumpToCode(String),
}

struct BlockPortSummary {
    name: String,
    type_name: Option<String>,
    is_conjugated: bool,
    direction: PortDirection,
}

struct BlockEditContext {
    owner_name: String,
    is_part_def: bool,
    specializes: String,
    attributes: Vec<(AttributeUsage, bool)>,
    ports: Vec<BlockPortSummary>,
    node_key: Option<String>,
}

struct ResolvedDetails {
    key: String,
    kind_label: String,
    kind_hint: Option<String>,
    qualified_name: String,
    short_name: String,
    source_file: String,
    origin_label: Option<String>,
    short_id: Option<String>,
    doc: Option<String>,
    type_name: Option<String>,
    multiplicity: Option<u32>,
    modifiers: ElementModifiers,
    supports_doc: bool,
    supports_short_id: bool,
    supports_type: bool,
    supports_def_usage: bool,
    supports_modifiers: bool,
    block_editor: Option<BlockEditContext>,
}

#[allow(clippy::too_many_arguments)]
pub fn show_inspector_panel(
    ui: &mut Ui,
    tab: &mut InspectorTab,
    selected: &SelectedTarget,
    draft: &mut InspectorDraft,
    graph: &DefinitionGraph,
    active_layout: Option<&LayoutOutput>,
    collab: &ProjectCollab,
    active_file: &str,
    locale: Locale,
) -> Option<InspectorAction> {
    ui.horizontal(|ui| {
        ui.selectable_value(tab, InspectorTab::Details, tr(locale, TextKey::TabDetails));
        ui.selectable_value(tab, InspectorTab::Code, tr(locale, TextKey::TabCodeEditor));
    });
    ui.separator();

    match tab {
        InspectorTab::Details => show_details_tab(ui, selected, draft, graph, active_layout, locale),
        InspectorTab::Code => show_code_tab(ui, graph, collab, active_file, locale),
    }
}

fn show_details_tab(
    ui: &mut Ui,
    selected: &SelectedTarget,
    draft: &mut InspectorDraft,
    graph: &DefinitionGraph,
    active_layout: Option<&LayoutOutput>,
    locale: Locale,
) -> Option<InspectorAction> {
    let mut action = None;

    ScrollArea::vertical().show(ui, |ui| match selected {
        SelectedTarget::None => {
            ui.add_space(12.0);
            ui.label(RichText::new(tr(locale, TextKey::DetailsSelectHint)).weak());
        }
        SelectedTarget::Edge {
            source,
            target,
            kind,
            trigger,
            guard,
            effect,
            file,
        } => {
            let key = format!(
                "edge:{source}->{target}:{kind:?}:{:?}:{:?}:{:?}",
                trigger, guard, effect
            );
            if draft.synced_key != key {
                draft.synced_key = key;
                draft.trigger = trigger.clone().unwrap_or_default();
                draft.guard = guard.clone().unwrap_or_default();
                draft.effect = effect.clone().unwrap_or_default();
            }

            ui.label(RichText::new(format!("Relationship: {kind:?}")).strong());
            ui.separator();
            ui.label(format!("From: {source}"));
            ui.label(format!("To:   {target}"));
            ui.label(format!("{}: {file}", tr(locale, TextKey::FieldSourceFile)));

            if *kind == RelationshipKind::Succession {
                ui.separator();
                ui.label(RichText::new(tr(locale, TextKey::FieldGuard)).strong());
                ui.text_edit_singleline(&mut draft.guard);
                ui.horizontal(|ui| {
                    if ui.button(tr(locale, TextKey::Apply)).clicked() {
                        action = Some(InspectorAction::ApplyCommand(
                            SyssurCommand::UpdateSuccessionGuard {
                                file: file.clone(),
                                source: source.clone(),
                                target: target.clone(),
                                new_guard: draft.guard.clone(),
                            },
                        ));
                    }
                    if ui.button(tr(locale, TextKey::ClearGuard)).clicked() {
                        draft.guard.clear();
                        action = Some(InspectorAction::ApplyCommand(
                            SyssurCommand::UpdateSuccessionGuard {
                                file: file.clone(),
                                source: source.clone(),
                                target: target.clone(),
                                new_guard: String::new(),
                            },
                        ));
                    }
                });
            } else if *kind == RelationshipKind::Transition {
                ui.separator();
                ui.label(RichText::new("Trigger (accept)").strong());
                ui.text_edit_singleline(&mut draft.trigger);
                ui.label(RichText::new(tr(locale, TextKey::FieldGuard)).strong());
                ui.text_edit_singleline(&mut draft.guard);
                ui.label(RichText::new("Effect (do)").strong());
                ui.text_edit_singleline(&mut draft.effect);
                if ui.button(tr(locale, TextKey::Apply)).clicked() {
                    action = Some(InspectorAction::ApplyCommand(
                        SyssurCommand::UpdateTransition {
                            file: file.clone(),
                            source: source.clone(),
                            target: target.clone(),
                            trigger: draft.trigger.clone(),
                            guard: draft.guard.clone(),
                            effect: draft.effect.clone(),
                        },
                    ));
                }
            }

            ui.separator();
            if ui.button(tr(locale, TextKey::OpenInCode)).clicked() {
                action = Some(InspectorAction::JumpToCode(file.clone()));
            }
        }
        SelectedTarget::Node { path, element_id } => {
            let resolved = resolve_details(graph, active_layout, element_id, Some(path));
            action = render_element_details(ui, resolved, draft, active_layout, locale);
        }
        SelectedTarget::Element(element_id) => {
            let resolved = resolve_details(graph, active_layout, element_id, None);
            action = render_element_details(ui, resolved, draft, active_layout, locale);
        }
    });

    action
}

fn render_element_details(
    ui: &mut Ui,
    resolved: Option<ResolvedDetails>,
    draft: &mut InspectorDraft,
    active_layout: Option<&LayoutOutput>,
    locale: Locale,
) -> Option<InspectorAction> {
    let Some(info) = resolved else {
        ui.label(RichText::new(tr(locale, TextKey::DetailsSelectHint)).weak());
        return None;
    };
    let mut action = None;

    if draft.synced_key != info.key {
        draft.synced_key = info.key.clone();
        draft.name = info.short_name.clone();
        draft.short_id = info.short_id.clone().unwrap_or_default();
        draft.doc = info.doc.clone().unwrap_or_default();
        draft.type_name = info.type_name.clone().unwrap_or_default();
        draft.multiplicity = info
            .multiplicity
            .map(|m| m.to_string())
            .unwrap_or_default();
        draft.value_expr = info.modifiers.value_expr.clone().unwrap_or_default();
        draft.attr_drafts.clear();
        if let Some(ref blk) = info.block_editor {
            draft.specializes = blk.specializes.clone();
            for (attr, _) in &blk.attributes {
                draft.attr_drafts.insert(
                    attr.name.clone(),
                    (
                        attr.name.clone(),
                        attr.type_name.clone().unwrap_or_default(),
                        attr.value_expr.clone().unwrap_or_default(),
                    ),
                );
            }
        } else {
            draft.specializes.clear();
        }
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(&info.kind_label)
                .strong()
                .color(Color32::from_rgb(110, 200, 245)),
        );
        if info.supports_def_usage {
            if ui.small_button("Def <-> Usage").clicked() {
                action = Some(InspectorAction::ApplyCommand(SyssurCommand::ToggleDefUsage {
                    file: info.source_file.clone(),
                    element_name: info.short_name.clone(),
                }));
            }
        }
    });
    if let Some(ref origin) = info.origin_label {
        ui.label(RichText::new(origin).small().weak());
    }
    ui.separator();

    // Name
    ui.label(RichText::new(tr(locale, TextKey::FieldName)).strong());
    ui.horizontal(|ui| {
        let resp = ui.add(
            TextEdit::singleline(&mut draft.name)
                .desired_width((ui.available_width() - 68.0).max(80.0)),
        );
        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button(tr(locale, TextKey::Apply)).clicked() || enter)
            && !draft.name.trim().is_empty()
            && draft.name.trim() != info.short_name
        {
            action = Some(InspectorAction::ApplyCommand(SyssurCommand::RenameElement {
                file: info.source_file.clone(),
                old_name: info.short_name.clone(),
                new_name: draft.name.trim().to_string(),
            }));
        }
    });

    // Short Name / ID
    if info.supports_short_id {
        ui.add_space(4.0);
        ui.label(RichText::new(tr(locale, TextKey::FieldShortId)).strong());
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut draft.short_id)
                    .desired_width((ui.available_width() - 68.0).max(80.0)),
            );
            if ui.button(tr(locale, TextKey::Apply)).clicked() {
                action = Some(InspectorAction::ApplyCommand(
                    SyssurCommand::UpdateElementShortId {
                        file: info.source_file.clone(),
                        element_name: info.short_name.clone(),
                        new_short_id: draft.short_id.trim().to_string(),
                    },
                ));
            }
        });
    }

    // Type & Multiplicity
    if info.supports_type {
        ui.add_space(4.0);
        ui.label(RichText::new(tr(locale, TextKey::FieldType)).strong());
        ui.text_edit_singleline(&mut draft.type_name);
        ui.label(RichText::new(tr(locale, TextKey::FieldMultiplicity)).strong());
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut draft.multiplicity).desired_width(72.0));
            if ui.button(tr(locale, TextKey::Apply)).clicked() {
                let mult = draft.multiplicity.trim().parse::<u32>().ok();
                action = Some(InspectorAction::ApplyCommand(
                    SyssurCommand::UpdatePartUsageType {
                        file: info.source_file.clone(),
                        part_name: info.short_name.clone(),
                        new_type: draft.type_name.trim().to_string(),
                        multiplicity: mult,
                    },
                ));
            }
        });
    }

    // Feature Value (= expr)
    if info.supports_modifiers {
        ui.add_space(4.0);
        ui.label(RichText::new("Value (= expr)").strong());
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut draft.value_expr)
                    .desired_width((ui.available_width() - 68.0).max(80.0)),
            );
            if ui.button(tr(locale, TextKey::Apply)).clicked() {
                action = Some(InspectorAction::ApplyCommand(
                    SyssurCommand::SetFeatureValue {
                        file: info.source_file.clone(),
                        element_name: info.short_name.clone(),
                        value_expr: draft.value_expr.clone(),
                    },
                ));
            }
        });
    }

    // Clause 7 Modifiers, Visibility & Direction
    if info.supports_modifiers {
        ui.add_space(4.0);
        ui.collapsing("SysML v2 Modifiers & Visibility", |ui| {
            ui.horizontal(|ui| {
                ui.label("Visibility:");
                let cur_vis = info.modifiers.visibility.as_deref().unwrap_or("default");
                for (label, val) in [
                    ("default", None),
                    ("public", Some("public")),
                    ("private", Some("private")),
                    ("protected", Some("protected")),
                ] {
                    if ui.selectable_label(cur_vis == label, label).clicked() {
                        action = Some(InspectorAction::ApplyCommand(SyssurCommand::SetVisibility {
                            file: info.source_file.clone(),
                            element_name: info.short_name.clone(),
                            visibility: val.map(str::to_string),
                        }));
                    }
                }
            });

            ui.horizontal(|ui| {
                ui.label("Direction:");
                for (label, dir_opt) in [
                    ("none", None),
                    ("in", Some(PortDirection::In)),
                    ("out", Some(PortDirection::Out)),
                    ("inout", Some(PortDirection::InOut)),
                ] {
                    if ui
                        .selectable_label(info.modifiers.direction == dir_opt, label)
                        .clicked()
                    {
                        action = Some(InspectorAction::ApplyCommand(SyssurCommand::SetDirection {
                            file: info.source_file.clone(),
                            element_name: info.short_name.clone(),
                            direction: dir_opt.map(|_| label.to_string()),
                        }));
                    }
                }
            });

            ui.horizontal_wrapped(|ui| {
                for (kw, active) in [
                    ("abstract", info.modifiers.is_abstract),
                    ("variation", info.modifiers.is_variation),
                    ("variant", info.modifiers.is_variant),
                    ("individual", info.modifiers.is_individual),
                    ("ref", info.modifiers.is_ref),
                    ("derived", info.modifiers.is_derived),
                    ("constant", info.modifiers.is_constant),
                    ("parallel", info.modifiers.is_parallel),
                ] {
                    if ui.selectable_label(active, kw).clicked() {
                        action = Some(InspectorAction::ApplyCommand(
                            SyssurCommand::ToggleModifier {
                                file: info.source_file.clone(),
                                element_name: info.short_name.clone(),
                                modifier: kw.to_string(),
                            },
                        ));
                    }
                }
            });
        });
    }

    // Documentation / Requirement text
    if info.supports_doc {
        ui.add_space(4.0);
        ui.label(RichText::new(tr(locale, TextKey::FieldDoc)).strong());
        ui.add_sized(
            [ui.available_width(), 82.0],
            TextEdit::multiline(&mut draft.doc),
        );
        if ui.button(tr(locale, TextKey::Apply)).clicked() {
            action = Some(InspectorAction::ApplyCommand(
                SyssurCommand::UpdateElementDoc {
                    file: info.source_file.clone(),
                    element_name: info.short_name.clone(),
                    new_doc: draft.doc.clone(),
                },
            ));
        }
    }

    // Block Properties, Attributes, Specialization & Port Repositioning
    if let Some(ref blk) = info.block_editor {
        if blk.is_part_def {
            ui.add_space(4.0);
            ui.label(RichText::new("Specializes (:>)").strong());
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut draft.specializes)
                        .hint_text("BasePartDef")
                        .desired_width((ui.available_width() - 68.0).max(80.0)),
                );
                if ui.button(tr(locale, TextKey::Apply)).clicked() {
                    action = Some(InspectorAction::ApplyCommand(
                        SyssurCommand::UpdatePartDefSpecializes {
                            file: info.source_file.clone(),
                            def_name: blk.owner_name.clone(),
                            specializes: draft.specializes.clone(),
                        },
                    ));
                }
            });
        }

        ui.add_space(4.0);
        egui::CollapsingHeader::new(format!("Attributes / Properties ({})", blk.attributes.len()))
            .default_open(true)
            .show(ui, |ui| {
                for (attr, is_inherited) in &blk.attributes {
                    let entry = draft
                        .attr_drafts
                        .entry(attr.name.clone())
                        .or_insert_with(|| {
                            (
                                attr.name.clone(),
                                attr.type_name.clone().unwrap_or_default(),
                                attr.value_expr.clone().unwrap_or_default(),
                            )
                        });
                    ui.push_id(&attr.name, |ui| {
                        ui.horizontal(|ui| {
                            if *is_inherited {
                                ui.label(RichText::new("^").small().color(Color32::from_rgb(140, 190, 240)));
                            }
                            ui.add(TextEdit::singleline(&mut entry.0).desired_width(72.0).hint_text("name"));
                            ui.label(":");
                            ui.add(TextEdit::singleline(&mut entry.1).desired_width(64.0).hint_text("Type"));
                            ui.label("=");
                            ui.add(TextEdit::singleline(&mut entry.2).desired_width(64.0).hint_text("val"));
                            if ui.small_button(tr(locale, TextKey::Apply)).clicked()
                                && !entry.0.trim().is_empty()
                            {
                                action = Some(InspectorAction::ApplyCommand(
                                    SyssurCommand::UpdateAttribute {
                                        file: info.source_file.clone(),
                                        owner_name: blk.owner_name.clone(),
                                        old_name: attr.name.clone(),
                                        new_name: entry.0.trim().to_string(),
                                        type_name: (!entry.1.trim().is_empty())
                                            .then(|| entry.1.trim().to_string()),
                                        value_expr: (!entry.2.trim().is_empty())
                                            .then(|| entry.2.trim().to_string()),
                                    },
                                ));
                            }
                            if !*is_inherited
                                && ui
                                    .small_button(
                                        RichText::new("×").color(Color32::from_rgb(245, 115, 115)),
                                    )
                                    .clicked()
                            {
                                action = Some(InspectorAction::ApplyCommand(
                                    SyssurCommand::DeleteAttribute {
                                        file: info.source_file.clone(),
                                        owner_name: blk.owner_name.clone(),
                                        attr_name: attr.name.clone(),
                                    },
                                ));
                            }
                        });
                    });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut draft.new_attr_name).desired_width(72.0).hint_text("newAttr"));
                    ui.add(TextEdit::singleline(&mut draft.new_attr_type).desired_width(64.0).hint_text("Real"));
                    ui.add(TextEdit::singleline(&mut draft.new_attr_value).desired_width(56.0).hint_text("0.0"));
                    if ui.small_button("+ Add").clicked() && !draft.new_attr_name.trim().is_empty()
                    {
                        action = Some(InspectorAction::ApplyCommand(
                            SyssurCommand::CreateAttribute {
                                file: info.source_file.clone(),
                                owner_name: blk.owner_name.clone(),
                                name: draft.new_attr_name.trim().to_string(),
                                type_name: (!draft.new_attr_type.trim().is_empty())
                                    .then(|| draft.new_attr_type.trim().to_string()),
                                value_expr: (!draft.new_attr_value.trim().is_empty())
                                    .then(|| draft.new_attr_value.trim().to_string()),
                            },
                        ));
                        draft.new_attr_name.clear();
                        draft.new_attr_type.clear();
                        draft.new_attr_value.clear();
                    }
                });
            });

        ui.add_space(4.0);
        egui::CollapsingHeader::new(format!("Ports ({})", blk.ports.len()))
            .default_open(!blk.ports.is_empty())
            .show(ui, |ui| {
                let pnode = blk
                    .node_key
                    .as_ref()
                    .and_then(|k| active_layout.and_then(|l| l.nodes.get(k)));
                for port in &blk.ports {
                    let positioned = pnode.and_then(|pn| pn.ports.iter().find(|pp| pp.name == port.name));
                    let cur_side = positioned.map(|pp| pp.side).unwrap_or(PortSide::Right);
                    let cur_off = positioned.map(|pp| pp.offset).unwrap_or(0.5);

                    ui.push_id(&port.name, |ui| {
                        ui.horizontal(|ui| {
                            let title = match port.type_name.as_deref() {
                                Some(tn) if port.is_conjugated => format!("{} : ~{tn}", port.name),
                                Some(tn) => format!("{} : {tn}", port.name),
                                None => format!("{} ({:?})", port.name, port.direction),
                            };
                            ui.label(RichText::new(title).monospace());
                            if ui.small_button("~").on_hover_text("Toggle conjugation").clicked() {
                                action = Some(InspectorAction::ApplyCommand(
                                    SyssurCommand::TogglePortConjugation {
                                        file: info.source_file.clone(),
                                        port_name: port.name.clone(),
                                    },
                                ));
                            }
                            if ui
                                .small_button(RichText::new("×").color(Color32::from_rgb(245, 115, 115)))
                                .on_hover_text("Delete port")
                                .clicked()
                            {
                                action = Some(InspectorAction::ApplyCommand(
                                    SyssurCommand::DeleteElement {
                                        file: info.source_file.clone(),
                                        kind_hint: Some("port".to_string()),
                                        name: port.name.clone(),
                                    },
                                ));
                            }
                        });

                        if let Some(ref nk) = blk.node_key {
                            ui.horizontal(|ui| {
                                for (lbl, side) in [
                                    ("T", PortSide::Top),
                                    ("R", PortSide::Right),
                                    ("B", PortSide::Bottom),
                                    ("L", PortSide::Left),
                                ] {
                                    if ui.selectable_label(cur_side == side, lbl).clicked() {
                                        action = Some(InspectorAction::MovePort {
                                            node_key: nk.clone(),
                                            port_name: port.name.clone(),
                                            placement: Some(PortPlacement {
                                                side,
                                                offset: cur_off,
                                            }),
                                        });
                                    }
                                }
                                if ui.small_button("Auto").clicked() {
                                    action = Some(InspectorAction::MovePort {
                                        node_key: nk.clone(),
                                        port_name: port.name.clone(),
                                        placement: None,
                                    });
                                }
                            });
                        }
                    });
                }

                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut draft.new_port_name).desired_width(72.0).hint_text("portName"));
                    ui.add(TextEdit::singleline(&mut draft.new_port_type).desired_width(68.0).hint_text("PortDef"));
                    ui.checkbox(&mut draft.new_port_conjugated, "~");
                    if ui.small_button("+ Add").clicked() && !draft.new_port_name.trim().is_empty()
                    {
                        let pdef = if draft.new_port_type.trim().is_empty() {
                            "Port".to_string()
                        } else {
                            draft.new_port_type.trim().to_string()
                        };
                        action = Some(InspectorAction::ApplyCommand(SyssurCommand::CreatePort {
                            file: info.source_file.clone(),
                            owner_def: blk.owner_name.clone(),
                            name: draft.new_port_name.trim().to_string(),
                            port_def: pdef,
                            conjugated: draft.new_port_conjugated,
                        }));
                        draft.new_port_name.clear();
                    }
                });
            });
    }

    ui.separator();
    ui.label(
        RichText::new(format!(
            "{}: {}",
            tr(locale, TextKey::FieldQualifiedName),
            info.qualified_name
        ))
        .small()
        .weak(),
    );
    ui.label(
        RichText::new(format!(
            "{}: {}",
            tr(locale, TextKey::FieldSourceFile),
            info.source_file
        ))
        .small()
        .weak(),
    );
    ui.horizontal(|ui| {
        if !info.source_file.is_empty() && ui.button(tr(locale, TextKey::OpenInCode)).clicked() {
            action = Some(InspectorAction::JumpToCode(info.source_file.clone()));
        }
        if ui
            .button(
                RichText::new(tr(locale, TextKey::DeleteElement))
                    .color(Color32::from_rgb(245, 115, 115)),
            )
            .clicked()
        {
            action = Some(InspectorAction::ApplyCommand(SyssurCommand::DeleteElement {
                file: info.source_file.clone(),
                kind_hint: info.kind_hint.clone(),
                name: info.short_name.clone(),
            }));
        }
    });

    action
}

fn resolve_details(
    graph: &DefinitionGraph,
    active_layout: Option<&LayoutOutput>,
    element_id: &ElementId,
    path: Option<&InstancePath>,
) -> Option<ResolvedDetails> {
    let short = element_id.short_name().to_string();
    let mods = graph
        .element_modifiers
        .get(element_id)
        .cloned()
        .unwrap_or_default();
    let resolved_node_key = path
        .map(InstancePath::canonical_key)
        .or_else(|| {
            active_layout.and_then(|layout| {
                layout
                    .nodes
                    .values()
                    .find(|n| n.path.leaf() == Some(element_id))
                    .map(|n| n.path.canonical_key())
            })
        });

    if let Some(req) = graph
        .requirements
        .get(element_id)
        .or_else(|| graph.requirements.values().find(|r| r.name == short))
    {
        return Some(ResolvedDetails {
            key: format!("req:{}:{:?}:{:?}", req.id, req.short_id, req.doc),
            kind_label: if req.is_usage {
                "Requirement Usage".to_string()
            } else {
                "Requirement Definition".to_string()
            },
            kind_hint: Some(if req.is_usage { "requirement" } else { "requirement def" }.into()),
            qualified_name: req.id.as_str().to_string(),
            short_name: req.name.clone(),
            source_file: req.span.file.clone(),
            origin_label: None,
            short_id: req.short_id.clone(),
            doc: req.doc.clone(),
            type_name: None,
            multiplicity: None,
            modifiers: graph
                .element_modifiers
                .get(&req.id)
                .cloned()
                .unwrap_or(mods),
            supports_doc: true,
            supports_short_id: true,
            supports_type: false,
            supports_def_usage: true,
            supports_modifiers: true,
            block_editor: None,
        });
    }

    if let Some(st) = find_state_recursive(graph, element_id) {
        return Some(ResolvedDetails {
            key: format!("state:{}:{:?}", st.id, st.doc),
            kind_label: if st.is_def {
                "State Definition".to_string()
            } else {
                "State Usage".to_string()
            },
            kind_hint: Some(if st.is_def { "state def" } else { "state" }.into()),
            qualified_name: st.id.as_str().to_string(),
            short_name: st.name.clone(),
            source_file: st.span.file.clone(),
            origin_label: None,
            short_id: None,
            doc: st.doc.clone(),
            type_name: None,
            multiplicity: None,
            modifiers: graph.element_modifiers.get(&st.id).cloned().unwrap_or(mods),
            supports_doc: true,
            supports_short_id: false,
            supports_type: false,
            supports_def_usage: true,
            supports_modifiers: true,
            block_editor: None,
        });
    }

    if let Some(act) = find_action_recursive(graph, element_id) {
        let kind_label = match act.kind {
            ActionNodeKind::Action => "Action",
            ActionNodeKind::Start => "Start Node",
            ActionNodeKind::Done => "Done Node",
            ActionNodeKind::Fork => "Fork Node",
            ActionNodeKind::Join => "Join Node",
            ActionNodeKind::Decide => "Decide Node",
            ActionNodeKind::Merge => "Merge Node",
        };
        return Some(ResolvedDetails {
            key: format!("act:{}:{:?}", act.id, act.doc),
            kind_label: kind_label.to_string(),
            kind_hint: None,
            qualified_name: act.id.as_str().to_string(),
            short_name: act.name.clone(),
            source_file: act.span.file.clone(),
            origin_label: None,
            short_id: None,
            doc: act.doc.clone(),
            type_name: None,
            multiplicity: None,
            modifiers: graph
                .element_modifiers
                .get(&act.id)
                .cloned()
                .unwrap_or(mods),
            supports_doc: act.kind == ActionNodeKind::Action,
            supports_short_id: false,
            supports_type: false,
            supports_def_usage: act.kind == ActionNodeKind::Action,
            supports_modifiers: act.kind == ActionNodeKind::Action,
            block_editor: None,
        });
    }

    if let Some(elem) = graph
        .generic_elements
        .get(element_id)
        .or_else(|| graph.generic_elements.values().find(|e| e.name == short))
    {
        return Some(ResolvedDetails {
            key: format!("elem:{}:{:?}:{:?}", elem.id, elem.doc, elem.short_id),
            kind_label: format!(
                "{:?}{}",
                elem.kind,
                if elem.is_def { " Def" } else { "" }
            ),
            kind_hint: None,
            qualified_name: elem.id.as_str().to_string(),
            short_name: elem.name.clone(),
            source_file: elem.span.file.clone(),
            origin_label: elem
                .type_ref
                .as_ref()
                .map(|t| format!(": {}", t.short_name())),
            short_id: elem.short_id.clone(),
            doc: elem.doc.clone(),
            type_name: elem.type_ref.as_ref().map(|t| t.short_name().to_string()),
            multiplicity: None,
            modifiers: graph
                .element_modifiers
                .get(&elem.id)
                .cloned()
                .unwrap_or(mods),
            supports_doc: true,
            supports_short_id: true,
            supports_type: false,
            supports_def_usage: true,
            supports_modifiers: true,
            block_editor: None,
        });
    }

    if let Some(pd) = graph.resolve_part_def(element_id) {
        if element_id == &pd.id {
            let eff_attrs: Vec<(AttributeUsage, bool)> = graph
                .effective_attributes_for_def(&pd.id)
                .into_iter()
                .map(|a| {
                    let inherited = matches!(a.origin, FeatureOrigin::Inherited(_));
                    (a, inherited)
                })
                .collect();
            let eff_ports: Vec<BlockPortSummary> = graph
                .effective_ports_for_def(&pd.id)
                .into_iter()
                .map(|p| BlockPortSummary {
                    name: p.name,
                    type_name: p.port_def.as_ref().map(|d| d.short_name().to_string()),
                    is_conjugated: p.is_conjugated,
                    direction: p.direction,
                })
                .collect();
            let specializes_str = pd
                .specializes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let attr_sig: Vec<String> = eff_attrs.iter().map(|(a, _)| a.display_label()).collect();
            let port_sig: Vec<String> = eff_ports
                .iter()
                .map(|p| format!("{}:{}", p.name, p.is_conjugated))
                .collect();
            return Some(ResolvedDetails {
                key: format!(
                    "partdef:{}:{}:{:?}:{:?}",
                    pd.id, specializes_str, attr_sig, port_sig
                ),
                kind_label: "Part Definition".to_string(),
                kind_hint: Some("part def".into()),
                qualified_name: pd.id.as_str().to_string(),
                short_name: pd.name.clone(),
                source_file: pd.span.file.clone(),
                origin_label: if pd.specializes.is_empty() {
                    None
                } else {
                    Some(format!(":> {specializes_str}"))
                },
                short_id: None,
                doc: None,
                type_name: None,
                multiplicity: None,
                modifiers: graph
                    .element_modifiers
                    .get(&pd.id)
                    .cloned()
                    .unwrap_or(mods),
                supports_doc: true,
                supports_short_id: false,
                supports_type: false,
                supports_def_usage: true,
                supports_modifiers: true,
                block_editor: Some(BlockEditContext {
                    owner_name: pd.name.clone(),
                    is_part_def: true,
                    specializes: specializes_str,
                    attributes: eff_attrs,
                    ports: eff_ports,
                    node_key: resolved_node_key,
                }),
            });
        }
    }

    if let Some(pu) = find_part_usage(graph, element_id) {
        let origin_label = match &pu.origin {
            FeatureOrigin::Owned => None,
            FeatureOrigin::Inherited(b) => Some(format!("^ Inherited from {}", b.short_name())),
            FeatureOrigin::Subsetted(t) => Some(format!(":> Subsets {}", t.short_name())),
            FeatureOrigin::Redefined(t) => Some(format!(":>> Redefines {}", t.short_name())),
        };
        let mut by_attr: BTreeMap<String, (AttributeUsage, bool)> = BTreeMap::new();
        if let Some(ref td) = pu.type_def {
            for attr in graph.effective_attributes_for_def(td) {
                by_attr.insert(attr.name.clone(), (attr, true));
            }
        }
        for local in &pu.attributes {
            let mut merged = local.clone();
            if let Some((prev, _)) = by_attr.get(&merged.name) {
                if merged.type_name.is_none() {
                    merged.type_name = prev.type_name.clone();
                }
                if merged.value_expr.is_none() {
                    merged.value_expr = prev.value_expr.clone();
                }
            }
            by_attr.insert(merged.name.clone(), (merged, false));
        }
        let merged_attrs: Vec<(AttributeUsage, bool)> = by_attr.into_values().collect();
        let eff_ports_raw = if pu.ports.is_empty() {
            pu.type_def
                .as_ref()
                .map(|td| graph.effective_ports_for_def(td))
                .unwrap_or_default()
        } else {
            pu.ports.clone()
        };
        let eff_ports: Vec<BlockPortSummary> = eff_ports_raw
            .into_iter()
            .map(|p| BlockPortSummary {
                name: p.name,
                type_name: p.port_def.as_ref().map(|d| d.short_name().to_string()),
                is_conjugated: p.is_conjugated,
                direction: p.direction,
            })
            .collect();
        let attr_sig: Vec<String> = merged_attrs
            .iter()
            .map(|(a, inh)| format!("{}({})", a.display_label(), inh))
            .collect();
        let port_sig: Vec<String> = eff_ports
            .iter()
            .map(|p| format!("{}:{}", p.name, p.is_conjugated))
            .collect();
        return Some(ResolvedDetails {
            key: format!(
                "part:{}:{:?}:{:?}:{:?}:{:?}",
                pu.id,
                pu.type_def.as_ref().map(|t| t.as_str()),
                pu.multiplicity,
                attr_sig,
                port_sig
            ),
            kind_label: "Part Usage".to_string(),
            kind_hint: Some("part".into()),
            qualified_name: path
                .map(ToString::to_string)
                .unwrap_or_else(|| pu.id.as_str().to_string()),
            short_name: pu.name.clone(),
            source_file: pu.span.file.clone(),
            origin_label,
            short_id: None,
            doc: None,
            type_name: pu.type_def.as_ref().map(|t| t.short_name().to_string()),
            multiplicity: pu.multiplicity,
            modifiers: graph
                .element_modifiers
                .get(&pu.id)
                .cloned()
                .unwrap_or(mods),
            supports_doc: true,
            supports_short_id: false,
            supports_type: true,
            supports_def_usage: true,
            supports_modifiers: true,
            block_editor: Some(BlockEditContext {
                owner_name: pu.name.clone(),
                is_part_def: false,
                specializes: String::new(),
                attributes: merged_attrs,
                ports: eff_ports,
                node_key: resolved_node_key,
            }),
        });
    }

    if let Some(pkg) = graph.packages.get(element_id) {
        return Some(ResolvedDetails {
            key: format!("pkg:{}", pkg.id),
            kind_label: "Package".to_string(),
            kind_hint: Some("package".into()),
            qualified_name: pkg.id.as_str().to_string(),
            short_name: pkg.name.clone(),
            source_file: pkg.span.file.clone(),
            origin_label: None,
            short_id: None,
            doc: None,
            type_name: None,
            multiplicity: None,
            modifiers: mods,
            supports_doc: true,
            supports_short_id: false,
            supports_type: false,
            supports_def_usage: false,
            supports_modifiers: false,
            block_editor: None,
        });
    }

    if let Some(view) = graph.views.get(element_id) {
        return Some(ResolvedDetails {
            key: format!("view:{}", view.id),
            kind_label: format!("{:?} View", view.kind),
            kind_hint: Some(if view.is_usage { "view" } else { "view def" }.into()),
            qualified_name: view.id.as_str().to_string(),
            short_name: view.name.clone(),
            source_file: view.span.file.clone(),
            origin_label: None,
            short_id: None,
            doc: None,
            type_name: None,
            multiplicity: None,
            modifiers: mods,
            supports_doc: false,
            supports_short_id: false,
            supports_type: false,
            supports_def_usage: true,
            supports_modifiers: false,
            block_editor: None,
        });
    }

    None
}

fn find_state_recursive<'a>(
    graph: &'a DefinitionGraph,
    target: &ElementId,
) -> Option<&'a StateNode> {
    fn search<'a>(node: &'a StateNode, target: &ElementId, short: &str) -> Option<&'a StateNode> {
        if &node.id == target || node.name == short {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = search(child, target, short) {
                return Some(found);
            }
        }
        None
    }
    let short = target.short_name();
    graph
        .states
        .values()
        .find_map(|root| search(root, target, short))
}

fn find_action_recursive<'a>(
    graph: &'a DefinitionGraph,
    target: &ElementId,
) -> Option<&'a ActionNode> {
    fn search<'a>(node: &'a ActionNode, target: &ElementId, short: &str) -> Option<&'a ActionNode> {
        if &node.id == target || node.name == short {
            return Some(node);
        }
        for child in &node.children {
            if let Some(found) = search(child, target, short) {
                return Some(found);
            }
        }
        None
    }
    let short = target.short_name();
    graph
        .actions
        .values()
        .find_map(|root| search(root, target, short))
}

fn find_part_usage<'a>(
    graph: &'a DefinitionGraph,
    target: &ElementId,
) -> Option<&'a syssur_model::PartUsage> {
    fn search_usage<'a>(
        u: &'a syssur_model::PartUsage,
        target: &ElementId,
        short: &str,
    ) -> Option<&'a syssur_model::PartUsage> {
        if &u.id == target || u.name == short {
            return Some(u);
        }
        for child in &u.nested_parts {
            if let Some(found) = search_usage(child, target, short) {
                return Some(found);
            }
        }
        None
    }

    let short = target.short_name();
    for u in graph.top_level_parts.values() {
        if let Some(found) = search_usage(u, target, short) {
            return Some(found);
        }
    }
    for pd in graph.part_defs.values() {
        for u in &pd.parts {
            if let Some(found) = search_usage(u, target, short) {
                return Some(found);
            }
        }
    }
    None
}

fn show_code_tab(
    ui: &mut Ui,
    graph: &DefinitionGraph,
    collab: &ProjectCollab,
    active_file: &str,
    locale: Locale,
) -> Option<InspectorAction> {
    let mut action = None;

    ui.horizontal_wrapped(|ui| {
        let filenames: Vec<String> = collab.files.keys().cloned().collect();
        for fname in filenames {
            if ui.selectable_label(active_file == fname, &fname).clicked() {
                action = Some(InspectorAction::SelectFile(fname));
            }
        }
        if ui.button(tr(locale, TextKey::Format)).clicked() {
            action = Some(InspectorAction::FormatFile {
                file: active_file.to_string(),
            });
        }
    });
    ui.separator();

    let diag_reserve = if graph.diagnostics.is_empty() {
        0.0
    } else {
        110.0
    };
    let editor_h = (ui.available_height() - diag_reserve).max(160.0);

    if let Some(doc) = collab.files.get(active_file) {
        let mut text = doc.text();
        let mut changed = false;
        ScrollArea::vertical()
            .id_salt("code_editor_scroll")
            .max_height(editor_h)
            .show(ui, |ui| {
                let resp = ui.add_sized(
                    [ui.available_width(), editor_h.max(240.0)],
                    TextEdit::multiline(&mut text).code_editor(),
                );
                changed = resp.changed();
            });
        if changed {
            action = Some(InspectorAction::EditFileText {
                file: active_file.to_string(),
                text,
            });
        }
    }

    if !graph.diagnostics.is_empty() {
        let error_count = graph
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        let warning_count = graph
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Errors: {error_count}"))
                    .color(if error_count > 0 {
                        Color32::from_rgb(245, 115, 115)
                    } else {
                        Color32::from_rgb(100, 210, 145)
                    })
                    .small()
                    .strong(),
            );
            ui.label(
                RichText::new(format!("| Warnings: {warning_count}"))
                    .color(if warning_count > 0 {
                        Color32::from_rgb(240, 190, 85)
                    } else {
                        Color32::from_rgb(150, 155, 165)
                    })
                    .small(),
            );
        });
        ScrollArea::vertical()
            .id_salt("code_diags_scroll")
            .max_height(110.0)
            .show(ui, |ui| {
                for diag in graph.diagnostics.iter().take(200) {
                    let (tag, color) = match diag.severity {
                        DiagnosticSeverity::Error => ("ERR", Color32::from_rgb(245, 115, 115)),
                        DiagnosticSeverity::Warning => ("WARN", Color32::from_rgb(240, 190, 85)),
                        DiagnosticSeverity::Info => ("INFO", Color32::from_rgb(130, 195, 255)),
                    };
                    ui.label(
                        RichText::new(format!("[{tag}] [{}] {}", diag.span.file, diag.message))
                            .color(color)
                            .small()
                            .monospace(),
                    );
                }
            });
    }

    action
}
