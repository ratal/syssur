//! Unified typed command engine (SolidCraft pattern) translating GUI, CLI,
//! and MCP actions into span-anchored `.sysml` text edits with undo/redo history.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextEdit {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

/// Formats a SysML v2 identifier: bare when `[A-Za-z_][A-Za-z0-9_]*`,
/// single-quoted (`'Doorbell System'`) otherwise.
pub fn format_sysml_name(name: &str) -> String {
    let trimmed = name.trim().trim_matches('\'');
    let mut chars = trimmed.chars();
    let bare_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if bare_ok {
        trimmed.to_string()
    } else {
        format!("'{}'", trimmed.replace('\'', "\\'"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyssurCommand {
    CreatePackage {
        file: String,
        name: String,
    },
    CreateView {
        file: String,
        package: String,
        name: String,
        view_kind: String,
        expose: Vec<String>,
    },
    ExposeInView {
        file: String,
        view_name: String,
        target_pattern: String,
    },
    CreatePartDef {
        file: String,
        package: String,
        name: String,
        specializes: Option<String>,
    },
    CreatePartUsage {
        file: String,
        owner_def: String,
        name: String,
        type_def: String,
    },
    CreatePort {
        file: String,
        owner_def: String,
        name: String,
        port_def: String,
        conjugated: bool,
    },
    CreateConnection {
        file: String,
        owner_def: String,
        source: String,
        target: String,
    },
    CreateInterface {
        file: String,
        owner_def: String,
        name: String,
        interface_def: String,
        source: String,
        target: String,
    },
    CreateAction {
        file: String,
        container: String,
        name: String,
        is_def: bool,
    },
    CreateControlNode {
        file: String,
        owner_action: String,
        kind: String,
        name: String,
    },
    CreateSuccession {
        file: String,
        owner_action: String,
        source: String,
        target: String,
        guard: Option<String>,
    },
    CreateRequirement {
        file: String,
        package: String,
        short_id: Option<String>,
        name: String,
        doc: String,
        is_usage: bool,
    },
    LinkSatisfy {
        file: String,
        package: String,
        requirement: String,
        by_target: String,
    },
    RenameElement {
        file: String,
        old_name: String,
        new_name: String,
    },
    UpdateElementDoc {
        file: String,
        element_name: String,
        new_doc: String,
    },
    UpdateElementShortId {
        file: String,
        element_name: String,
        new_short_id: String,
    },
    UpdateSuccessionGuard {
        file: String,
        source: String,
        target: String,
        new_guard: String,
    },
    UpdatePartUsageType {
        file: String,
        part_name: String,
        new_type: String,
        multiplicity: Option<u32>,
    },
    DeleteElement {
        file: String,
        kind_hint: Option<String>,
        name: String,
    },
    CreateSysmlElement {
        file: String,
        container: String,
        keyword: String,
        name: String,
        is_def: bool,
        specializes: Option<String>,
        type_ref: Option<String>,
        doc: Option<String>,
    },
    CreateStateTransition {
        file: String,
        owner_state: String,
        source: String,
        target: String,
        trigger: Option<String>,
        guard: Option<String>,
        effect: Option<String>,
    },
    CreateRelationship {
        file: String,
        owner_scope: String,
        kind: String,
        source: String,
        target: String,
        label: Option<String>,
        guard: Option<String>,
    },
    ToggleModifier {
        file: String,
        element_name: String,
        modifier: String,
    },
    SetVisibility {
        file: String,
        element_name: String,
        visibility: Option<String>,
    },
    SetDirection {
        file: String,
        element_name: String,
        direction: Option<String>,
    },
    TogglePortConjugation {
        file: String,
        port_name: String,
    },
    ToggleDefUsage {
        file: String,
        element_name: String,
    },
    ChangeElementKind {
        file: String,
        element_name: String,
        new_keyword: String,
    },
    ExtractDefinition {
        file: String,
        package: String,
        usage_name: String,
        def_name: String,
    },
    MoveElementToPackage {
        file: String,
        element_name: String,
        target_package: String,
    },
    SetFeatureValue {
        file: String,
        element_name: String,
        value_expr: String,
    },
    UpdateTransition {
        file: String,
        source: String,
        target: String,
        trigger: String,
        guard: String,
        effect: String,
    },
    CreateAttribute {
        file: String,
        owner_name: String,
        name: String,
        type_name: Option<String>,
        value_expr: Option<String>,
    },
    UpdateAttribute {
        file: String,
        owner_name: String,
        old_name: String,
        new_name: String,
        type_name: Option<String>,
        value_expr: Option<String>,
    },
    DeleteAttribute {
        file: String,
        owner_name: String,
        attr_name: String,
    },
    UpdatePartDefSpecializes {
        file: String,
        def_name: String,
        specializes: String,
    },
}

impl SyssurCommand {
    pub fn summary(&self) -> String {
        match self {
            Self::CreatePackage { name, .. } => format!("package {name}"),
            Self::CreateView { name, view_kind, .. } => format!("view {name} : {view_kind}"),
            Self::ExposeInView { view_name, target_pattern, .. } => {
                format!("{view_name}: expose {target_pattern}")
            }
            Self::CreatePartDef { name, .. } => format!("part def {name}"),
            Self::CreatePartUsage { owner_def, name, type_def, .. } => {
                format!("{owner_def}: part {name} : {type_def}")
            }
            Self::CreatePort { owner_def, name, port_def, conjugated, .. } => {
                let tilde = if *conjugated { "~" } else { "" };
                format!("{owner_def}: port {name} : {tilde}{port_def}")
            }
            Self::CreateConnection { source, target, .. } => format!("connect {source} to {target}"),
            Self::CreateInterface { name, interface_def, .. } => {
                format!("interface {name} : {interface_def}")
            }
            Self::CreateAction { container, name, is_def, .. } => {
                let kw = if *is_def { "action def" } else { "action" };
                format!("{container}: {kw} {name}")
            }
            Self::CreateControlNode { owner_action, kind, name, .. } => {
                format!("{owner_action}: {kind} {name}")
            }
            Self::CreateSuccession { source, target, guard, .. } => match guard {
                Some(g) => format!("first {source} if {g} then {target}"),
                None => format!("first {source} then {target}"),
            },
            Self::CreateRequirement { name, is_usage, .. } => {
                let kw = if *is_usage { "requirement" } else { "requirement def" };
                format!("{kw} {name}")
            }
            Self::LinkSatisfy { requirement, by_target, .. } => {
                format!("satisfy {requirement} by {by_target}")
            }
            Self::RenameElement { old_name, new_name, .. } => {
                format!("rename {old_name} -> {new_name}")
            }
            Self::UpdateElementDoc { element_name, .. } => {
                format!("update doc on {element_name}")
            }
            Self::UpdateElementShortId { element_name, new_short_id, .. } => {
                format!("set id <'{new_short_id}'> on {element_name}")
            }
            Self::UpdateSuccessionGuard { source, target, new_guard, .. } => {
                if new_guard.trim().is_empty() {
                    format!("clear guard {source} -> {target}")
                } else {
                    format!("guard {source} -> {target}: {new_guard}")
                }
            }
            Self::UpdatePartUsageType { part_name, new_type, .. } => {
                format!("set {part_name} : {new_type}")
            }
            Self::DeleteElement { name, .. } => format!("delete {name}"),
            Self::CreateSysmlElement { keyword, is_def, name, .. } => {
                let d = if *is_def { " def" } else { "" };
                format!("{keyword}{d} {name}")
            }
            Self::CreateStateTransition { source, target, .. } => {
                format!("transition {source} -> {target}")
            }
            Self::CreateRelationship { kind, source, target, .. } => {
                format!("{kind} {source} -> {target}")
            }
            Self::ToggleModifier { element_name, modifier, .. } => {
                format!("toggle {modifier} on {element_name}")
            }
            Self::SetVisibility { element_name, visibility, .. } => {
                format!("visibility {:?} on {element_name}", visibility)
            }
            Self::SetDirection { element_name, direction, .. } => {
                format!("direction {:?} on {element_name}", direction)
            }
            Self::TogglePortConjugation { port_name, .. } => {
                format!("toggle ~ on {port_name}")
            }
            Self::ToggleDefUsage { element_name, .. } => {
                format!("toggle def/usage on {element_name}")
            }
            Self::ChangeElementKind { element_name, new_keyword, .. } => {
                format!("convert {element_name} to {new_keyword}")
            }
            Self::ExtractDefinition { usage_name, def_name, .. } => {
                format!("extract def {def_name} from {usage_name}")
            }
            Self::MoveElementToPackage { element_name, target_package, .. } => {
                format!("move {element_name} -> {target_package}")
            }
            Self::SetFeatureValue { element_name, value_expr, .. } => {
                format!("set {element_name} = {value_expr}")
            }
            Self::UpdateTransition { source, target, .. } => {
                format!("update transition {source} -> {target}")
            }
            Self::CreateAttribute { owner_name, name, .. } => {
                format!("{owner_name}: attribute {name}")
            }
            Self::UpdateAttribute { owner_name, old_name, new_name, .. } => {
                if old_name == new_name {
                    format!("{owner_name}: update attribute {new_name}")
                } else {
                    format!("{owner_name}: attribute {old_name} -> {new_name}")
                }
            }
            Self::DeleteAttribute { owner_name, attr_name, .. } => {
                format!("{owner_name}: delete attribute {attr_name}")
            }
            Self::UpdatePartDefSpecializes { def_name, specializes, .. } => {
                if specializes.trim().is_empty() {
                    format!("clear specialization on {def_name}")
                } else {
                    format!("{def_name} :> {specializes}")
                }
            }
        }
    }

    pub fn target_file(&self) -> &str {
        match self {
            Self::CreatePackage { file, .. }
            | Self::CreateView { file, .. }
            | Self::ExposeInView { file, .. }
            | Self::CreatePartDef { file, .. }
            | Self::CreatePartUsage { file, .. }
            | Self::CreatePort { file, .. }
            | Self::CreateConnection { file, .. }
            | Self::CreateInterface { file, .. }
            | Self::CreateAction { file, .. }
            | Self::CreateControlNode { file, .. }
            | Self::CreateSuccession { file, .. }
            | Self::CreateRequirement { file, .. }
            | Self::LinkSatisfy { file, .. }
            | Self::RenameElement { file, .. }
            | Self::UpdateElementDoc { file, .. }
            | Self::UpdateElementShortId { file, .. }
            | Self::UpdateSuccessionGuard { file, .. }
            | Self::UpdatePartUsageType { file, .. }
            | Self::DeleteElement { file, .. }
            | Self::CreateSysmlElement { file, .. }
            | Self::CreateStateTransition { file, .. }
            | Self::CreateRelationship { file, .. }
            | Self::ToggleModifier { file, .. }
            | Self::SetVisibility { file, .. }
            | Self::SetDirection { file, .. }
            | Self::TogglePortConjugation { file, .. }
            | Self::ToggleDefUsage { file, .. }
            | Self::ChangeElementKind { file, .. }
            | Self::ExtractDefinition { file, .. }
            | Self::MoveElementToPackage { file, .. }
            | Self::SetFeatureValue { file, .. }
            | Self::UpdateTransition { file, .. }
            | Self::CreateAttribute { file, .. }
            | Self::UpdateAttribute { file, .. }
            | Self::DeleteAttribute { file, .. }
            | Self::UpdatePartDefSpecializes { file, .. } => file,
        }
    }
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("target file '{0}' not found")]
    MissingFile(String),
    #[error("container '{0}' not found in '{1}'")]
    MissingContainer(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub summary: String,
    pub file: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Default)]
pub struct CommandHistory {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
}

impl CommandHistory {
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.undo_stack
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn record(&mut self, entry: HistoryEntry) {
        self.undo_stack.push(entry);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> Option<HistoryEntry> {
        let entry = self.undo_stack.pop()?;
        self.redo_stack.push(entry.clone());
        Some(entry)
    }

    pub fn redo(&mut self) -> Option<HistoryEntry> {
        let entry = self.redo_stack.pop()?;
        self.undo_stack.push(entry.clone());
        Some(entry)
    }
}

/// Compiles a `SyssurCommand` into a span-anchored `TextEdit` against the current `.sysml` text.
pub fn plan_text_edit(cmd: &SyssurCommand, current_text: &str) -> Result<TextEdit, CommandError> {
    let file = cmd.target_file().to_string();
    match cmd {
        SyssurCommand::CreatePackage { name, .. } => {
            let insert_at = current_text.len();
            let prefix = if current_text.ends_with('\n') || current_text.is_empty() {
                ""
            } else {
                "\n"
            };
            let fname = format_sysml_name(name);
            Ok(TextEdit {
                file,
                start: insert_at,
                end: insert_at,
                replacement: format!(
                    "{prefix}\npackage {fname} {{\n    import StandardViewDefinitions::*;\n}}\n"
                ),
            })
        }
        SyssurCommand::CreateView {
            package,
            name,
            view_kind,
            expose,
            ..
        } => {
            let fname = format_sysml_name(name);
            let mut body_lines = String::new();
            for pat in expose {
                body_lines.push_str(&format!("        expose {pat};\n"));
            }
            let stmt = format!("    view {fname} : {view_kind} {{\n{body_lines}    }}\n");
            plan_insert_into_container(&file, current_text, &["package"], package, &stmt)
        }
        SyssurCommand::ExposeInView {
            view_name,
            target_pattern,
            ..
        } => {
            let stmt = format!("        expose {target_pattern};\n");
            plan_insert_into_container(
                &file,
                current_text,
                &["view def", "view"],
                view_name,
                &stmt,
            )
        }
        SyssurCommand::CreatePartDef {
            package,
            name,
            specializes,
            ..
        } => {
            let fname = format_sysml_name(name);
            let spec = specializes
                .as_ref()
                .map(|s| format!(" :> {}", format_sysml_name(s)))
                .unwrap_or_default();
            let stmt = format!("    part def {fname}{spec} {{\n    }}\n");
            plan_insert_into_container(&file, current_text, &["package"], package, &stmt)
        }
        SyssurCommand::CreatePartUsage {
            owner_def,
            name,
            type_def,
            ..
        } => {
            let fname = format_sysml_name(name);
            let stmt = if type_def.trim().is_empty() {
                format!("        part {fname};\n")
            } else {
                format!("        part {fname} : {};\n", format_sysml_name(type_def))
            };
            plan_insert_into_container(
                &file,
                current_text,
                &["part def", "part", "package"],
                owner_def,
                &stmt,
            )
        }
        SyssurCommand::CreatePort {
            owner_def,
            name,
            port_def,
            conjugated,
            ..
        } => {
            let fname = format_sysml_name(name);
            let tilde = if *conjugated { "~" } else { "" };
            let stmt = format!(
                "        port {fname} : {tilde}{};\n",
                format_sysml_name(port_def)
            );
            plan_insert_into_container(
                &file,
                current_text,
                &["part def", "part"],
                owner_def,
                &stmt,
            )
        }
        SyssurCommand::CreateConnection {
            owner_def,
            source,
            target,
            ..
        } => {
            let stmt = format!("        connect {source} to {target};\n");
            plan_insert_into_container(
                &file,
                current_text,
                &["part def", "part", "package"],
                owner_def,
                &stmt,
            )
        }
        SyssurCommand::CreateInterface {
            owner_def,
            name,
            interface_def,
            source,
            target,
            ..
        } => {
            let fname = format_sysml_name(name);
            let stmt = format!(
                "        interface {fname} : {}\n            connect {source} to {target};\n",
                format_sysml_name(interface_def)
            );
            plan_insert_into_container(
                &file,
                current_text,
                &["part def", "part", "package"],
                owner_def,
                &stmt,
            )
        }
        SyssurCommand::CreateAction {
            container,
            name,
            is_def,
            ..
        } => {
            let fname = format_sysml_name(name);
            let kw = if *is_def { "action def" } else { "action" };
            let stmt = format!("    {kw} {fname} {{\n    }}\n");
            plan_insert_into_container(
                &file,
                current_text,
                &["action def", "action", "package"],
                container,
                &stmt,
            )
        }
        SyssurCommand::CreateControlNode {
            owner_action,
            kind,
            name,
            ..
        } => {
            let stmt = match kind.as_str() {
                "start" => "        first start;\n".to_string(),
                "done" => "        // done terminal\n".to_string(),
                _ => format!("        {kind} {};\n", format_sysml_name(name)),
            };
            plan_insert_into_container(
                &file,
                current_text,
                &["action def", "action"],
                owner_action,
                &stmt,
            )
        }
        SyssurCommand::CreateSuccession {
            owner_action,
            source,
            target,
            guard,
            ..
        } => {
            let src = if source == "start" {
                "start".to_string()
            } else {
                format_sysml_name(source)
            };
            let dst = if target == "done" {
                "done".to_string()
            } else {
                format_sysml_name(target)
            };
            let stmt = match guard.as_deref().map(str::trim).filter(|g| !g.is_empty()) {
                Some(g) => format!("        first {src} if {g} then {dst};\n"),
                None => format!("        first {src} then {dst};\n"),
            };
            plan_insert_into_container(
                &file,
                current_text,
                &["action def", "action"],
                owner_action,
                &stmt,
            )
        }
        SyssurCommand::CreateRequirement {
            package,
            short_id,
            name,
            doc,
            is_usage,
            ..
        } => {
            let kw = if *is_usage {
                "requirement"
            } else {
                "requirement def"
            };
            let fname = format_sysml_name(name);
            let short = short_id
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .map(|s| format!("<'{}'> ", s.trim()))
                .unwrap_or_default();
            let stmt = if doc.trim().is_empty() {
                format!("    {kw} {short}{fname} {{\n    }}\n")
            } else {
                format!(
                    "    {kw} {short}{fname} {{\n        doc /* {} */\n    }}\n",
                    doc.trim()
                )
            };
            plan_insert_into_container(&file, current_text, &["package"], package, &stmt)
        }
        SyssurCommand::LinkSatisfy {
            package,
            requirement,
            by_target,
            ..
        } => {
            let req = format_sysml_name(requirement);
            let stmt = format!("    satisfy {req} by {by_target};\n");
            plan_insert_into_container(
                &file,
                current_text,
                &["package", "part def", "part"],
                package,
                &stmt,
            )
        }
        SyssurCommand::RenameElement {
            old_name, new_name, ..
        } => {
            let replaced = replace_sysml_name_tokens(current_text, old_name, new_name);
            Ok(TextEdit {
                file,
                start: 0,
                end: current_text.len(),
                replacement: replaced,
            })
        }
        SyssurCommand::UpdateElementDoc {
            element_name,
            new_doc,
            ..
        } => update_element_doc_edit(&file, current_text, element_name, new_doc),
        SyssurCommand::UpdateElementShortId {
            element_name,
            new_short_id,
            ..
        } => update_element_short_id_edit(&file, current_text, element_name, new_short_id),
        SyssurCommand::UpdateSuccessionGuard {
            source,
            target,
            new_guard,
            ..
        } => update_succession_guard_edit(&file, current_text, source, target, new_guard),
        SyssurCommand::UpdatePartUsageType {
            part_name,
            new_type,
            multiplicity,
            ..
        } => update_part_type_edit(&file, current_text, part_name, new_type, *multiplicity),
        SyssurCommand::DeleteElement {
            kind_hint, name, ..
        } => delete_element_edit(&file, current_text, kind_hint.as_deref(), name),
        SyssurCommand::CreateSysmlElement {
            container,
            keyword,
            name,
            is_def,
            specializes,
            type_ref,
            doc,
            ..
        } => create_sysml_element_edit(
            &file,
            current_text,
            container,
            keyword,
            name,
            *is_def,
            specializes.as_deref(),
            type_ref.as_deref(),
            doc.as_deref(),
        ),
        SyssurCommand::CreateStateTransition {
            owner_state,
            source,
            target,
            trigger,
            guard,
            effect,
            ..
        } => {
            let src = if source == "start" {
                "start".to_string()
            } else {
                format_sysml_name(source)
            };
            let dst = if target == "done" {
                "done".to_string()
            } else {
                format_sysml_name(target)
            };
            let trig = trigger
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|t| format!(" accept {t}"))
                .unwrap_or_default();
            let grd = guard
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|g| format!(" if {g}"))
                .unwrap_or_default();
            let eff = effect
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|e| format!(" do {e}"))
                .unwrap_or_default();
            let stmt = format!("        transition first {src}{trig}{grd}{eff} then {dst};\n");
            plan_insert_into_container(
                &file,
                current_text,
                &["state def", "state", "package"],
                owner_state,
                &stmt,
            )
        }
        SyssurCommand::CreateRelationship {
            owner_scope,
            kind,
            source,
            target,
            label,
            guard,
            ..
        } => create_relationship_edit(
            &file,
            current_text,
            owner_scope,
            kind,
            source,
            target,
            label.as_deref(),
            guard.as_deref(),
        ),
        SyssurCommand::ToggleModifier {
            element_name,
            modifier,
            ..
        } => toggle_modifier_edit(&file, current_text, element_name, modifier),
        SyssurCommand::SetVisibility {
            element_name,
            visibility,
            ..
        } => set_visibility_edit(&file, current_text, element_name, visibility.as_deref()),
        SyssurCommand::SetDirection {
            element_name,
            direction,
            ..
        } => set_direction_edit(&file, current_text, element_name, direction.as_deref()),
        SyssurCommand::TogglePortConjugation { port_name, .. } => {
            toggle_port_conjugation_edit(&file, current_text, port_name)
        }
        SyssurCommand::ToggleDefUsage { element_name, .. } => {
            toggle_def_usage_edit(&file, current_text, element_name)
        }
        SyssurCommand::ChangeElementKind {
            element_name,
            new_keyword,
            ..
        } => change_element_kind_edit(&file, current_text, element_name, new_keyword),
        SyssurCommand::ExtractDefinition {
            package,
            usage_name,
            def_name,
            ..
        } => extract_definition_edit(&file, current_text, package, usage_name, def_name),
        SyssurCommand::MoveElementToPackage {
            element_name,
            target_package,
            ..
        } => move_element_to_package_edit(&file, current_text, element_name, target_package),
        SyssurCommand::SetFeatureValue {
            element_name,
            value_expr,
            ..
        } => set_feature_value_edit(&file, current_text, element_name, value_expr),
        SyssurCommand::UpdateTransition {
            source,
            target,
            trigger,
            guard,
            effect,
            ..
        } => update_transition_edit(&file, current_text, source, target, trigger, guard, effect),
        SyssurCommand::CreateAttribute {
            owner_name,
            name,
            type_name,
            value_expr,
            ..
        } => {
            let stmt = format_attribute_stmt("        ", false, name, type_name.as_deref(), value_expr.as_deref());
            plan_insert_into_container(
                &file,
                current_text,
                &["part def", "part", "item def", "item"],
                owner_name,
                &stmt,
            )
        }
        SyssurCommand::UpdateAttribute {
            owner_name,
            old_name,
            new_name,
            type_name,
            value_expr,
            ..
        } => update_attribute_edit(
            &file,
            current_text,
            owner_name,
            old_name,
            new_name,
            type_name.as_deref(),
            value_expr.as_deref(),
        ),
        SyssurCommand::DeleteAttribute {
            owner_name,
            attr_name,
            ..
        } => delete_attribute_edit(&file, current_text, owner_name, attr_name),
        SyssurCommand::UpdatePartDefSpecializes {
            def_name,
            specializes,
            ..
        } => update_part_def_specializes_edit(&file, current_text, def_name, specializes),
    }
}

fn format_attribute_stmt(
    indent: &str,
    redefines: bool,
    name: &str,
    type_name: Option<&str>,
    value_expr: Option<&str>,
) -> String {
    let fname = format_sysml_name(name);
    let mut stmt = if redefines {
        format!("{indent}attribute redefines {fname}")
    } else {
        format!("{indent}attribute {fname}")
    };
    if let Some(ty) = type_name.map(str::trim).filter(|t| !t.is_empty()) {
        if ty.starts_with(":>") || ty.starts_with(":>>") {
            stmt.push(' ');
            stmt.push_str(ty);
        } else {
            stmt.push_str(" : ");
            stmt.push_str(ty);
        }
    }
    if let Some(val) = value_expr.map(str::trim).filter(|v| !v.is_empty()) {
        stmt.push_str(" = ");
        stmt.push_str(val);
    }
    stmt.push_str(";\n");
    stmt
}

fn find_container_inner_span(
    text: &str,
    kind_prefixes: &[&str],
    owner_name: &str,
) -> Option<(usize, usize)> {
    let header_idx = find_header_index(text, kind_prefixes, owner_name)?;
    let rest = &text[header_idx..];
    let brace_rel = rest.find('{')?;
    if let Some(semi_rel) = rest.find(';') {
        if semi_rel < brace_rel {
            return None;
        }
    }
    let open_abs = header_idx + brace_rel;
    let mut depth = 0usize;
    for (rel, ch) in text[open_abs..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((open_abs + 1, open_abs + rel));
                }
            }
            _ => {}
        }
    }
    None
}

/// Finds a direct-child `attribute ...;` line matching `attr_name` at brace depth 0 inside `owner_name`.
fn find_attr_line_in_owner(
    text: &str,
    owner_name: &str,
    attr_name: &str,
) -> Option<(usize, usize, String, bool)> {
    let (inner_start, inner_end) =
        find_container_inner_span(text, &["part def", "part", "item def", "item"], owner_name)?;
    let short = attr_name.trim().trim_matches('\'');
    let mut depth = 0usize;
    let mut pos = inner_start;
    for line in text[inner_start..inner_end].split_inclusive('\n') {
        let line_start = pos;
        let line_end = pos + line.len();
        pos = line_end;
        let trimmed = line.trim();
        let at_top = depth == 0;
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        if !at_top {
            continue;
        }
        let body = trimmed.split("//").next().unwrap_or("").trim();
        if !body.contains("attribute ") && !body.starts_with("attribute\t") {
            continue;
        }
        if !body.ends_with(';') || body.contains('{') {
            continue;
        }
        let matches_name = body.contains(&format!("'{short}'"))
            || body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .any(|tok| tok == short);
        if matches_name {
            let indent: String = line
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let is_redef = body.contains("redefines ") || body.contains(":>>");
            return Some((line_start, line_end, indent, is_redef));
        }
    }
    None
}

fn update_attribute_edit(
    file: &str,
    text: &str,
    owner_name: &str,
    old_name: &str,
    new_name: &str,
    type_name: Option<&str>,
    value_expr: Option<&str>,
) -> Result<TextEdit, CommandError> {
    if let Some((start, end, indent, is_redef)) = find_attr_line_in_owner(text, owner_name, old_name)
    {
        let mut replacement = format_attribute_stmt(&indent, is_redef, new_name, type_name, value_expr);
        if !text[start..end].ends_with('\n') && replacement.ends_with('\n') {
            replacement.pop();
        }
        return Ok(TextEdit {
            file: file.to_string(),
            start,
            end,
            replacement,
        });
    }
    // Overriding an inherited attribute locally on a PartDef/PartUsage: insert `attribute redefines`.
    let stmt = format_attribute_stmt("        ", true, new_name, type_name, value_expr);
    plan_insert_into_container(
        file,
        text,
        &["part def", "part", "item def", "item"],
        owner_name,
        &stmt,
    )
}

fn delete_attribute_edit(
    file: &str,
    text: &str,
    owner_name: &str,
    attr_name: &str,
) -> Result<TextEdit, CommandError> {
    if let Some((start, end, _, _)) = find_attr_line_in_owner(text, owner_name, attr_name) {
        return Ok(TextEdit {
            file: file.to_string(),
            start,
            end,
            replacement: String::new(),
        });
    }
    Err(CommandError::MissingContainer(
        format!("{owner_name}::{attr_name}"),
        file.to_string(),
    ))
}

fn update_part_def_specializes_edit(
    file: &str,
    text: &str,
    def_name: &str,
    specializes: &str,
) -> Result<TextEdit, CommandError> {
    let header_idx = find_header_index(text, &["part def"], def_name)
        .ok_or_else(|| CommandError::MissingContainer(def_name.to_string(), file.to_string()))?;
    let rest = &text[header_idx..];
    let term_rel = rest
        .find('{')
        .into_iter()
        .chain(rest.find(';'))
        .min()
        .ok_or_else(|| CommandError::MissingContainer(def_name.to_string(), file.to_string()))?;
    let term_abs = header_idx + term_rel;
    let term_ch = text.as_bytes()[term_abs] as char;
    let header_slice = &text[header_idx..term_abs];

    let spec_pos = header_slice
        .find(":>")
        .or_else(|| header_slice.find(" specializes "));
    let clean_spec = specializes.trim();
    let replacement = match spec_pos {
        Some(rel) => {
            let prefix = header_slice[..rel].trim_end();
            if clean_spec.is_empty() {
                if term_ch == '{' {
                    format!("{prefix} ")
                } else {
                    prefix.to_string()
                }
            } else if term_ch == '{' {
                format!("{prefix} :> {clean_spec} ")
            } else {
                format!("{prefix} :> {clean_spec}")
            }
        }
        None => {
            let prefix = header_slice.trim_end();
            if clean_spec.is_empty() {
                header_slice.to_string()
            } else if term_ch == '{' {
                format!("{prefix} :> {clean_spec} ")
            } else {
                format!("{prefix} :> {clean_spec}")
            }
        }
    };

    Ok(TextEdit {
        file: file.to_string(),
        start: header_idx,
        end: term_abs,
        replacement,
    })
}

pub fn apply_text_edit(text: &str, edit: &TextEdit) -> String {
    let mut out = String::with_capacity(text.len() + edit.replacement.len());
    out.push_str(&text[..edit.start]);
    out.push_str(&edit.replacement);
    out.push_str(&text[edit.end..]);
    out
}

const ALL_CONTAINER_PREFIXES: &[&str] = &[
    "library package",
    "package",
    "part def",
    "part",
    "port def",
    "port",
    "action def",
    "action",
    "state def",
    "state",
    "requirement def",
    "requirement",
    "constraint def",
    "constraint",
    "concern def",
    "concern",
    "calc def",
    "calc",
    "use case def",
    "use case",
    "analysis def",
    "analysis",
    "verification def",
    "verification",
    "case def",
    "case",
    "item def",
    "item",
    "attribute def",
    "attribute",
    "enum def",
    "enum",
    "occurrence def",
    "occurrence",
    "connection def",
    "connection",
    "interface def",
    "interface",
    "allocation def",
    "allocation",
    "flow def",
    "flow",
    "view def",
    "view",
    "viewpoint def",
    "viewpoint",
    "rendering def",
    "rendering",
    "metadata def",
    "metadata",
    "fork",
    "join",
    "decide",
    "merge",
    "comment",
    "alias",
];

fn find_header_index(text: &str, kind_prefixes: &[&str], name: &str) -> Option<usize> {
    let formatted = format_sysml_name(name);
    let quoted = format!("'{}'", name.trim().trim_matches('\''));
    for prefix in kind_prefixes {
        for cand in [formatted.as_str(), quoted.as_str(), name.trim()] {
            let pat = format!("{prefix} {cand}");
            if let Some(idx) = find_whole_header(text, &pat) {
                return Some(idx);
            }
        }
    }
    // Also support declarations with `<'ID'>` or modifiers before the name
    let short_target = name.trim().trim_matches('\'');
    for prefix in kind_prefixes {
        let mut search_from = 0;
        while let Some(rel) = text[search_from..].find(prefix) {
            let abs = search_from + rel;
            let prev_ok = text[..abs]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace());
            let after_kw = abs + prefix.len();
            let kw_end_ok = text[after_kw..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace());
            if prev_ok && kw_end_ok {
                let line_end = text[abs..].find('\n').map(|n| abs + n).unwrap_or(text.len());
                let header_slice = &text[abs..line_end];
                if header_slice.contains(&format!("'{short_target}'"))
                    || header_slice
                        .split_whitespace()
                        .any(|tok| tok.trim_end_matches(['{', ';', ':', '[']) == short_target)
                {
                    return Some(abs);
                }
            }
            search_from = abs + prefix.len();
        }
    }
    None
}

fn find_whole_header(text: &str, pat: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = text[from..].find(pat) {
        let idx = from + rel;
        let prev_ok = text[..idx]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace());
        let after = idx + pat.len();
        let next_ok = text[after..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || matches!(c, '{' | ';' | ':' | '['));
        if prev_ok && next_ok {
            return Some(idx);
        }
        from = idx + pat.len();
    }
    None
}

fn plan_insert_into_container(
    file: &str,
    text: &str,
    kind_prefixes: &[&str],
    container_name: &str,
    stmt: &str,
) -> Result<TextEdit, CommandError> {
    let header_idx = find_header_index(text, kind_prefixes, container_name)
        .ok_or_else(|| CommandError::MissingContainer(container_name.to_string(), file.to_string()))?;

    let rest = &text[header_idx..];
    let brace_rel = rest.find('{');
    let semi_rel = rest.find(';');

    // If semicolon comes before '{' (or there is no '{'), expand `...;` into `... {\n<stmt><indent>}`
    let is_semicolon_leaf = match (semi_rel, brace_rel) {
        (Some(s), Some(b)) => s < b,
        (Some(_), None) => true,
        _ => false,
    };

    if is_semicolon_leaf {
        let semi_abs = header_idx + semi_rel.unwrap();
        let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent: String = text[line_start..header_idx]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        return Ok(TextEdit {
            file: file.to_string(),
            start: semi_abs,
            end: semi_abs + 1,
            replacement: format!(" {{\n{stmt}{indent}}}"),
        });
    }

    let open_rel = brace_rel
        .ok_or_else(|| CommandError::MissingContainer(container_name.to_string(), file.to_string()))?;
    let mut depth = 0usize;
    for (rel, ch) in text[header_idx + open_rel..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let abs = header_idx + open_rel + rel;
                    let line_start = text[..abs].rfind('\n').map(|i| i + 1).unwrap_or(abs);
                    let insert_at = if text[line_start..abs]
                        .chars()
                        .all(|c| c == ' ' || c == '\t')
                    {
                        line_start
                    } else {
                        abs
                    };
                    return Ok(TextEdit {
                        file: file.to_string(),
                        start: insert_at,
                        end: insert_at,
                        replacement: stmt.to_string(),
                    });
                }
            }
            _ => {}
        }
    }

    Err(CommandError::MissingContainer(
        container_name.to_string(),
        file.to_string(),
    ))
}

fn find_element_block_span(
    text: &str,
    kind_hint: Option<&str>,
    name: &str,
) -> Option<(usize, usize)> {
    let header_idx = if let Some(hint) = kind_hint {
        find_header_index(text, &[hint], name)
            .or_else(|| find_header_index(text, ALL_CONTAINER_PREFIXES, name))?
    } else {
        find_header_index(text, ALL_CONTAINER_PREFIXES, name)?
    };
    let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rest = &text[header_idx..];
    let brace_rel = rest.find('{');
    let semi_rel = rest.find(';');

    let is_semi = match (semi_rel, brace_rel) {
        (Some(s), Some(b)) => s < b,
        (Some(_), None) => true,
        _ => false,
    };

    let mut end = if is_semi {
        header_idx + semi_rel? + 1
    } else if let Some(open_rel) = brace_rel {
        let mut depth = 0usize;
        let mut close_abs = None;
        for (rel, ch) in text[header_idx + open_rel..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close_abs = Some(header_idx + open_rel + rel + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        close_abs?
    } else {
        text[header_idx..]
            .find('\n')
            .map(|n| header_idx + n)
            .unwrap_or(text.len())
    };

    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    }
    Some((line_start, end))
}

fn delete_element_edit(
    file: &str,
    text: &str,
    kind_hint: Option<&str>,
    name: &str,
) -> Result<TextEdit, CommandError> {
    let (start, end) = find_element_block_span(text, kind_hint, name)
        .ok_or_else(|| CommandError::MissingContainer(name.to_string(), file.to_string()))?;
    Ok(TextEdit {
        file: file.to_string(),
        start,
        end,
        replacement: String::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn create_sysml_element_edit(
    file: &str,
    text: &str,
    container: &str,
    keyword: &str,
    name: &str,
    is_def: bool,
    specializes: Option<&str>,
    type_ref: Option<&str>,
    doc: Option<&str>,
) -> Result<TextEdit, CommandError> {
    let fname = format_sysml_name(name);
    let kw = keyword.trim();

    if kw == "package" || kw == "library package" {
        let insert_at = text.len();
        let prefix = if text.ends_with('\n') || text.is_empty() {
            ""
        } else {
            "\n"
        };
        return Ok(TextEdit {
            file: file.to_string(),
            start: insert_at,
            end: insert_at,
            replacement: format!(
                "{prefix}\n{kw} {fname} {{\n    import StandardViewDefinitions::*;\n}}\n"
            ),
        });
    }

    let stmt = match kw {
        "comment" => {
            let body = doc.unwrap_or("Comment text");
            format!("    comment {fname} /* {body} */\n")
        }
        "import" => format!("    import {fname}::*;\n"),
        "alias" => {
            let tgt = type_ref.unwrap_or("Target");
            format!("    alias {fname} for {tgt};\n")
        }
        "filter" => format!("    filter {name};\n"),
        "perform action" => format!("        perform action {fname};\n"),
        "exhibit state" => format!("        exhibit state {fname};\n"),
        "include use case" => format!("        include use case {fname};\n"),
        "assert constraint" => format!("        assert constraint {fname};\n"),
        "accept" => format!("        accept {fname} : Event;\n"),
        "send" => format!("        send {fname}();\n"),
        "assign" => format!("        assign {fname} := true;\n"),
        "terminate" => "        terminate;\n".to_string(),
        "if" => format!("        if true {{ action {fname}; }}\n"),
        "while" | "loop" => format!("        while true {{ action {fname}; }}\n"),
        "for" => format!("        for i in 1..2 {{ action {fname}; }}\n"),
        "subject" | "actor" | "stakeholder" => {
            let ty = type_ref
                .filter(|t| !t.trim().is_empty())
                .map(|t| format!(" : {}", format_sysml_name(t)))
                .unwrap_or_default();
            format!("        {kw} {fname}{ty};\n")
        }
        "event" | "timeslice" | "snapshot" => {
            let prefix = if kw == "event" {
                "event occurrence"
            } else {
                kw
            };
            format!("        {prefix} {fname};\n")
        }
        "ref" => {
            let ty = type_ref
                .filter(|t| !t.trim().is_empty())
                .map(|t| format!(" : {}", format_sysml_name(t)))
                .unwrap_or_default();
            format!("        ref {fname}{ty};\n")
        }
        "verification" => {
            let def_kw = if is_def { "verification def" } else { "verification" };
            let doc_line = doc
                .filter(|d| !d.trim().is_empty())
                .map(|d| format!("        doc /* {} */\n", d.trim()))
                .unwrap_or_default();
            format!("    {def_kw} {fname} {{\n{doc_line}        objective {{ }}\n    }}\n")
        }
        "enum" if is_def => {
            format!("    enum def {fname} {{\n        enum optionA;\n        enum optionB;\n    }}\n")
        }
        _ => {
            let def_part = if is_def { " def" } else { "" };
            let spec_part = specializes
                .filter(|s| !s.trim().is_empty())
                .map(|s| format!(" :> {}", format_sysml_name(s)))
                .unwrap_or_default();
            let type_part = type_ref
                .filter(|t| !t.trim().is_empty())
                .map(|t| format!(" : {}", format_sysml_name(t)))
                .unwrap_or_default();
            if let Some(d) = doc.filter(|d| !d.trim().is_empty()) {
                format!(
                    "    {kw}{def_part} {fname}{spec_part}{type_part} {{\n        doc /* {} */\n    }}\n",
                    d.trim()
                )
            } else if is_def
                || matches!(
                    kw,
                    "part" | "action" | "state" | "case" | "use case" | "analysis" | "requirement"
                )
            {
                format!("    {kw}{def_part} {fname}{spec_part}{type_part} {{\n    }}\n")
            } else {
                format!("    {kw}{def_part} {fname}{spec_part}{type_part};\n")
            }
        }
    };

    plan_insert_into_container(file, text, ALL_CONTAINER_PREFIXES, container, &stmt)
}

#[allow(clippy::too_many_arguments)]
fn create_relationship_edit(
    file: &str,
    text: &str,
    owner_scope: &str,
    kind: &str,
    source: &str,
    target: &str,
    label: Option<&str>,
    guard: Option<&str>,
) -> Result<TextEdit, CommandError> {
    let src_fmt = if source == "start" {
        "start".to_string()
    } else {
        format_sysml_name(source)
    };
    let dst_fmt = if target == "done" {
        "done".to_string()
    } else {
        format_sysml_name(target)
    };

    match kind {
        "specialization" | "subsetting" | "redefinition" | "reference_subsetting" | "typing" => {
            let op = match kind {
                "redefinition" => ":>>",
                "reference_subsetting" => "::>",
                "typing" => ":",
                _ => ":>",
            };
            return add_specialization_clause(file, text, source, op, &dst_fmt);
        }
        _ => {}
    }

    let stmt = match kind {
        "connection" => format!("        connect {src_fmt} to {dst_fmt};\n"),
        "interface" => {
            let lname = label
                .filter(|l| !l.trim().is_empty())
                .map(format_sysml_name)
                .unwrap_or_else(|| "link".to_string());
            format!("        interface {lname} connect {src_fmt} to {dst_fmt};\n")
        }
        "binding" => format!("        bind {src_fmt} = {dst_fmt};\n"),
        "flow" => format!("        flow from {src_fmt} to {dst_fmt};\n"),
        "allocate" => format!("        allocate {src_fmt} to {dst_fmt};\n"),
        "dependency" => format!("    dependency from {src_fmt} to {dst_fmt};\n"),
        "satisfy" => format!("    satisfy {dst_fmt} by {src_fmt};\n"),
        "verify" => format!("        objective {{ verify {dst_fmt}; }}\n"),
        "transition" => {
            let grd = guard
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(|g| format!(" if {g}"))
                .unwrap_or_default();
            format!("        transition first {src_fmt}{grd} then {dst_fmt};\n")
        }
        "succession" | "guarded_succession" => {
            let grd = guard
                .map(str::trim)
                .filter(|g| !g.is_empty())
                .map(|g| format!(" if {g}"))
                .unwrap_or_default();
            format!("        first {src_fmt}{grd} then {dst_fmt};\n")
        }
        _ => format!("        connect {src_fmt} to {dst_fmt};\n"),
    };

    plan_insert_into_container(file, text, ALL_CONTAINER_PREFIXES, owner_scope, &stmt)
}

fn add_specialization_clause(
    file: &str,
    text: &str,
    element_name: &str,
    op: &str,
    target: &str,
) -> Result<TextEdit, CommandError> {
    let header_idx = find_header_index(text, ALL_CONTAINER_PREFIXES, element_name)
        .ok_or_else(|| CommandError::MissingContainer(element_name.to_string(), file.to_string()))?;
    let line_end = text[header_idx..]
        .find('\n')
        .map(|n| header_idx + n)
        .unwrap_or(text.len());
    let header_line = &text[header_idx..line_end];
    let insert_pos = header_line
        .find('{')
        .or_else(|| header_line.find(';'))
        .map(|rel| header_idx + rel)
        .unwrap_or(line_end);
    let prefix = if text[..insert_pos].ends_with(' ') {
        ""
    } else {
        " "
    };
    Ok(TextEdit {
        file: file.to_string(),
        start: insert_pos,
        end: insert_pos,
        replacement: format!("{prefix}{op} {target} "),
    })
}

fn header_line_bounds(
    file: &str,
    text: &str,
    element_name: &str,
) -> Result<(usize, usize, usize), CommandError> {
    let header_idx = find_header_index(text, ALL_CONTAINER_PREFIXES, element_name)
        .ok_or_else(|| CommandError::MissingContainer(element_name.to_string(), file.to_string()))?;
    let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[header_idx..]
        .find('\n')
        .map(|n| header_idx + n)
        .unwrap_or(text.len());
    Ok((line_start, header_idx, line_end))
}

fn toggle_modifier_edit(
    file: &str,
    text: &str,
    element_name: &str,
    modifier: &str,
) -> Result<TextEdit, CommandError> {
    let (line_start, _header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let line = &text[line_start..line_end];
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let body = line[indent.len()..].to_string();

    if modifier == "parallel" {
        if let Some(pos) = body.find(" parallel") {
            let mut new_body = body.clone();
            new_body.replace_range(pos..pos + " parallel".len(), "");
            return Ok(TextEdit {
                file: file.to_string(),
                start: line_start,
                end: line_end,
                replacement: format!("{indent}{new_body}"),
            });
        } else if let Some(bpos) = body.find('{') {
            let mut new_body = body.clone();
            let ins = if bpos > 0 && !body[..bpos].ends_with(' ') {
                " parallel "
            } else {
                "parallel "
            };
            new_body.insert_str(bpos, ins);
            return Ok(TextEdit {
                file: file.to_string(),
                start: line_start,
                end: line_end,
                replacement: format!("{indent}{new_body}"),
            });
        }
    }

    let tokens: Vec<&str> = body.split_whitespace().collect();
    let new_tokens: Vec<&str> = if tokens.contains(&modifier) {
        tokens.into_iter().filter(|t| *t != modifier).collect()
    } else {
        let vis_pos = tokens
            .iter()
            .position(|t| matches!(*t, "public" | "private" | "protected"))
            .map(|p| p + 1)
            .unwrap_or(0);
        let mut out = tokens;
        out.insert(vis_pos, modifier);
        out
    };
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: format!("{indent}{}", new_tokens.join(" ")),
    })
}

fn set_visibility_edit(
    file: &str,
    text: &str,
    element_name: &str,
    visibility: Option<&str>,
) -> Result<TextEdit, CommandError> {
    let (line_start, _header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let line = &text[line_start..line_end];
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut tokens: Vec<&str> = line[indent.len()..]
        .split_whitespace()
        .filter(|t| !matches!(*t, "public" | "private" | "protected"))
        .collect();
    if let Some(vis) = visibility.filter(|v| !v.is_empty()) {
        tokens.insert(0, vis);
    }
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: format!("{indent}{}", tokens.join(" ")),
    })
}

fn set_direction_edit(
    file: &str,
    text: &str,
    element_name: &str,
    direction: Option<&str>,
) -> Result<TextEdit, CommandError> {
    let (line_start, _header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let line = &text[line_start..line_end];
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let mut tokens: Vec<&str> = line[indent.len()..]
        .split_whitespace()
        .filter(|t| !matches!(*t, "in" | "out" | "inout"))
        .collect();
    if let Some(dir) = direction.filter(|d| !d.is_empty()) {
        let vis_pos = tokens
            .iter()
            .position(|t| matches!(*t, "public" | "private" | "protected"))
            .map(|p| p + 1)
            .unwrap_or(0);
        tokens.insert(vis_pos, dir);
    }
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: format!("{indent}{}", tokens.join(" ")),
    })
}

fn toggle_port_conjugation_edit(
    file: &str,
    text: &str,
    port_name: &str,
) -> Result<TextEdit, CommandError> {
    let (line_start, _header_idx, line_end) = header_line_bounds(file, text, port_name)?;
    let line = &text[line_start..line_end];
    let replacement = if let Some(pos) = line.find(": ~") {
        let mut s = line.to_string();
        s.replace_range(pos..pos + 3, ": ");
        s
    } else if let Some(pos) = line.find(": ") {
        let mut s = line.to_string();
        s.replace_range(pos..pos + 2, ": ~");
        s
    } else {
        line.to_string()
    };
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement,
    })
}

fn toggle_def_usage_edit(
    file: &str,
    text: &str,
    element_name: &str,
) -> Result<TextEdit, CommandError> {
    let (line_start, header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let rest = &text[header_idx..line_end];
    let mut new_rest = rest.to_string();
    let mut toggled = false;
    for kw in [
        "use case",
        "requirement",
        "constraint",
        "connection",
        "interface",
        "allocation",
        "occurrence",
        "attribute",
        "verification",
        "analysis",
        "viewpoint",
        "rendering",
        "metadata",
        "concern",
        "action",
        "state",
        "part",
        "port",
        "item",
        "enum",
        "flow",
        "calc",
        "case",
        "view",
    ] {
        let def_pat = format!("{kw} def ");
        if new_rest.starts_with(&def_pat) {
            new_rest.replace_range(..def_pat.len(), &format!("{kw} "));
            toggled = true;
            break;
        }
        let use_pat = format!("{kw} ");
        if new_rest.starts_with(&use_pat) {
            new_rest.replace_range(..use_pat.len(), &def_pat);
            toggled = true;
            break;
        }
    }
    if !toggled {
        return Err(CommandError::MissingContainer(
            element_name.to_string(),
            file.to_string(),
        ));
    }
    let indent_and_prefix = &text[line_start..header_idx];
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: format!("{indent_and_prefix}{new_rest}"),
    })
}

fn change_element_kind_edit(
    file: &str,
    text: &str,
    element_name: &str,
    new_keyword: &str,
) -> Result<TextEdit, CommandError> {
    let (line_start, header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let rest = &text[header_idx..line_end];
    for prefix in ALL_CONTAINER_PREFIXES {
        let pat = format!("{prefix} ");
        if rest.starts_with(&pat) {
            let is_def = prefix.ends_with(" def");
            let target_kw = if is_def && !new_keyword.ends_with(" def") {
                format!("{} def", new_keyword.trim())
            } else {
                new_keyword.trim().to_string()
            };
            let after = &rest[prefix.len()..];
            let indent_and_prefix = &text[line_start..header_idx];
            return Ok(TextEdit {
                file: file.to_string(),
                start: line_start,
                end: line_end,
                replacement: format!("{indent_and_prefix}{target_kw}{after}"),
            });
        }
    }
    Err(CommandError::MissingContainer(
        element_name.to_string(),
        file.to_string(),
    ))
}

fn extract_definition_edit(
    file: &str,
    text: &str,
    package: &str,
    usage_name: &str,
    def_name: &str,
) -> Result<TextEdit, CommandError> {
    let (_line_start, header_idx, line_end) = header_line_bounds(file, text, usage_name)?;
    let rest = &text[header_idx..line_end];
    let kind_kw = rest.split_whitespace().next().unwrap_or("part");
    let def_stmt = format!("    {kind_kw} def {} {{\n    }}\n", format_sysml_name(def_name));
    let step1 = plan_insert_into_container(file, text, &["package"], package, &def_stmt)?;
    let mid = apply_text_edit(text, &step1);
    let step2 = add_specialization_clause(file, &mid, usage_name, ":", &format_sysml_name(def_name))?;
    let final_text = apply_text_edit(&mid, &step2);
    Ok(TextEdit {
        file: file.to_string(),
        start: 0,
        end: text.len(),
        replacement: final_text,
    })
}

fn move_element_to_package_edit(
    file: &str,
    text: &str,
    element_name: &str,
    target_package: &str,
) -> Result<TextEdit, CommandError> {
    let (start, end) = find_element_block_span(text, None, element_name)
        .ok_or_else(|| CommandError::MissingContainer(element_name.to_string(), file.to_string()))?;
    let snippet = text[start..end].to_string();
    let without = format!("{}{}", &text[..start], &text[end..]);
    let insert_edit = plan_insert_into_container(
        file,
        &without,
        &["library package", "package"],
        target_package,
        &snippet,
    )?;
    let final_text = apply_text_edit(&without, &insert_edit);
    Ok(TextEdit {
        file: file.to_string(),
        start: 0,
        end: text.len(),
        replacement: final_text,
    })
}

fn set_feature_value_edit(
    file: &str,
    text: &str,
    element_name: &str,
    value_expr: &str,
) -> Result<TextEdit, CommandError> {
    let (line_start, header_idx, line_end) = header_line_bounds(file, text, element_name)?;
    let expr = value_expr.trim();
    let head = &text[line_start..header_idx];
    let rest = &text[header_idx..line_end];
    let term_rel = rest
        .find('{')
        .or_else(|| rest.find(';'))
        .unwrap_or(rest.len());
    let before_term = &rest[..term_rel];
    let term_tail = &rest[term_rel..];

    let base_before = match before_term.find(" = ") {
        Some(eq) => before_term[..eq].trim_end(),
        None => before_term.trim_end(),
    };
    let updated = if expr.is_empty() {
        format!("{head}{base_before}{term_tail}")
    } else if term_tail.starts_with('{') {
        format!("{head}{base_before} = {expr} {term_tail}")
    } else {
        format!("{head}{base_before} = {expr}{term_tail}")
    };
    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: updated,
    })
}

fn update_transition_edit(
    file: &str,
    text: &str,
    source: &str,
    target: &str,
    trigger: &str,
    guard: &str,
    effect: &str,
) -> Result<TextEdit, CommandError> {
    let src_fmt = if source == "start" {
        "start".to_string()
    } else {
        format_sysml_name(source)
    };
    let dst_fmt = if target == "done" {
        "done".to_string()
    } else {
        format_sysml_name(target)
    };

    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with("transition ") && trimmed.contains(&src_fmt) && trimmed.contains(&dst_fmt) {
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let ending = if line.ends_with('\n') { "\n" } else { "" };
            let trig = if trigger.trim().is_empty() {
                String::new()
            } else {
                format!(" accept {}", trigger.trim())
            };
            let grd = if guard.trim().is_empty() {
                String::new()
            } else {
                format!(" if {}", guard.trim())
            };
            let eff = if effect.trim().is_empty() {
                String::new()
            } else {
                format!(" do {}", effect.trim())
            };
            return Ok(TextEdit {
                file: file.to_string(),
                start: offset,
                end: offset + line.len(),
                replacement: format!("{indent}transition first {src_fmt}{trig}{grd}{eff} then {dst_fmt};{ending}"),
            });
        }
        offset += line.len();
    }

    Err(CommandError::MissingContainer(
        format!("transition {source} -> {target}"),
        file.to_string(),
    ))
}

fn update_element_doc_edit(
    file: &str,
    text: &str,
    element_name: &str,
    new_doc: &str,
) -> Result<TextEdit, CommandError> {
    let header_idx = find_header_index(text, ALL_CONTAINER_PREFIXES, element_name)
        .ok_or_else(|| CommandError::MissingContainer(element_name.to_string(), file.to_string()))?;
    let rest = &text[header_idx..];
    let brace_rel = rest.find('{');
    let semi_rel = rest.find(';');
    let is_semi = match (semi_rel, brace_rel) {
        (Some(s), Some(b)) => s < b,
        (Some(_), None) => true,
        _ => false,
    };

    let clean_doc = new_doc.trim().replace("*/", "* /");
    if is_semi {
        let semi_abs = header_idx + semi_rel.unwrap();
        let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent: String = text[line_start..header_idx]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        return Ok(TextEdit {
            file: file.to_string(),
            start: semi_abs,
            end: semi_abs + 1,
            replacement: format!(" {{\n{indent}    doc /* {clean_doc} */\n{indent}}}"),
        });
    }

    let open_abs = header_idx
        + brace_rel.ok_or_else(|| {
            CommandError::MissingContainer(element_name.to_string(), file.to_string())
        })?;
    let mut depth = 0usize;
    let mut close_abs = text.len();
    for (rel, ch) in text[open_abs..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close_abs = open_abs + rel;
                    break;
                }
            }
            _ => {}
        }
    }

    let body = &text[open_abs + 1..close_abs];
    if let Some(doc_rel) = body.find("doc /*") {
        if let Some(end_rel) = body[doc_rel..].find("*/") {
            let abs_start = open_abs + 1 + doc_rel;
            let abs_end = abs_start + end_rel + 2;
            return Ok(TextEdit {
                file: file.to_string(),
                start: abs_start,
                end: abs_end,
                replacement: format!("doc /* {clean_doc} */"),
            });
        }
    }

    let insert_at = open_abs + 1;
    let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let indent: String = text[line_start..header_idx]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect();
    Ok(TextEdit {
        file: file.to_string(),
        start: insert_at,
        end: insert_at,
        replacement: format!("\n{indent}    doc /* {clean_doc} */"),
    })
}

fn update_element_short_id_edit(
    file: &str,
    text: &str,
    element_name: &str,
    new_short_id: &str,
) -> Result<TextEdit, CommandError> {
    let prefixes = ["requirement def", "requirement", "part def", "part"];
    let header_idx = find_header_index(text, &prefixes, element_name)
        .ok_or_else(|| CommandError::MissingContainer(element_name.to_string(), file.to_string()))?;
    let line_end = text[header_idx..]
        .find('\n')
        .map(|n| header_idx + n)
        .unwrap_or(text.len());
    let header_line = &text[header_idx..line_end];
    let clean_id = new_short_id.trim();

    if let (Some(lt), Some(gt)) = (header_line.find('<'), header_line.find('>')) {
        if lt < gt {
            let abs_start = header_idx + lt;
            let abs_end = header_idx + gt + 1;
            let replacement = if clean_id.is_empty() {
                String::new()
            } else {
                format!("<'{clean_id}'>")
            };
            return Ok(TextEdit {
                file: file.to_string(),
                start: abs_start,
                end: abs_end,
                replacement,
            });
        }
    }

    if let Some(sp) = header_line.find(' ') {
        // Insert after keyword (handling `requirement def` too)
        let kw_end = if header_line.starts_with("requirement def ") {
            "requirement def ".len()
        } else if header_line.starts_with("part def ") {
            "part def ".len()
        } else {
            sp + 1
        };
        let insert_at = header_idx + kw_end;
        return Ok(TextEdit {
            file: file.to_string(),
            start: insert_at,
            end: insert_at,
            replacement: if clean_id.is_empty() {
                String::new()
            } else {
                format!("<'{clean_id}'> ")
            },
        });
    }

    Err(CommandError::MissingContainer(
        element_name.to_string(),
        file.to_string(),
    ))
}

fn update_succession_guard_edit(
    file: &str,
    text: &str,
    source: &str,
    target: &str,
    new_guard: &str,
) -> Result<TextEdit, CommandError> {
    let src_fmt = if source == "start" {
        "start".to_string()
    } else {
        format_sysml_name(source)
    };
    let dst_fmt = if target == "done" {
        "done".to_string()
    } else {
        format_sysml_name(target)
    };
    let guard_trimmed = new_guard.trim();

    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let has_first = trimmed.starts_with("first ") || trimmed.starts_with("succession ");
        if has_first && trimmed.contains(&src_fmt) && trimmed.contains(&dst_fmt) {
            let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let ending = if line.ends_with('\n') { "\n" } else { "" };
            let replacement = if guard_trimmed.is_empty() {
                format!("{indent}first {src_fmt} then {dst_fmt};{ending}")
            } else {
                format!("{indent}first {src_fmt} if {guard_trimmed} then {dst_fmt};{ending}")
            };
            return Ok(TextEdit {
                file: file.to_string(),
                start: offset,
                end: offset + line.len(),
                replacement,
            });
        }
        offset += line.len();
    }

    Err(CommandError::MissingContainer(
        format!("{source} -> {target}"),
        file.to_string(),
    ))
}

fn update_part_type_edit(
    file: &str,
    text: &str,
    part_name: &str,
    new_type: &str,
    multiplicity: Option<u32>,
) -> Result<TextEdit, CommandError> {
    let header_idx = find_header_index(text, &["part"], part_name)
        .ok_or_else(|| CommandError::MissingContainer(part_name.to_string(), file.to_string()))?;
    let line_start = text[..header_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[header_idx..]
        .find('\n')
        .map(|n| header_idx + n)
        .unwrap_or(text.len());
    let line = &text[line_start..line_end];
    let indent: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let suffix = if line.trim_end().ends_with('{') {
        " {"
    } else {
        ";"
    };
    let pname = format_sysml_name(part_name);
    let type_part = if new_type.trim().is_empty() {
        String::new()
    } else {
        format!(" : {}", format_sysml_name(new_type))
    };
    let mult_part = multiplicity
        .map(|m| format!(" [{m}]"))
        .unwrap_or_default();

    Ok(TextEdit {
        file: file.to_string(),
        start: line_start,
        end: line_end,
        replacement: format!("{indent}part {pname}{type_part}{mult_part}{suffix}"),
    })
}

fn replace_sysml_name_tokens(text: &str, old_name: &str, new_name: &str) -> String {
    let clean_old = old_name.trim().trim_matches('\'');
    let clean_new = new_name.trim().trim_matches('\'');
    if clean_old.is_empty() || clean_old == clean_new {
        return text.to_string();
    }
    let formatted_new = format_sysml_name(clean_new);
    let quoted_old = format!("'{clean_old}'");

    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with(&quoted_old) {
            out.push_str(&formatted_new);
            i += quoted_old.len();
            continue;
        }
        if text[i..].starts_with(clean_old) {
            let prev_ok = text[..i].chars().next_back().is_none_or(|c| !is_ident(c));
            let after_idx = i + clean_old.len();
            let next_ok = text[after_idx..]
                .chars()
                .next()
                .is_none_or(|c| !is_ident(c));
            if prev_ok && next_ok {
                out.push_str(&formatted_new);
                i = after_idx;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_inserts_part_usage_and_undo_restores_exact_source() {
        let initial = "package P {\n    part def Rover {\n    }\n}\n";
        let cmd = SyssurCommand::CreatePartUsage {
            file: "rover.sysml".into(),
            owner_def: "Rover".into(),
            name: "sensor".into(),
            type_def: "Lidar".into(),
        };
        let edit = plan_text_edit(&cmd, initial).unwrap();
        let updated = apply_text_edit(initial, &edit);
        assert!(updated.contains("part sensor : Lidar;"));

        let mut history = CommandHistory::default();
        history.record(HistoryEntry {
            summary: cmd.summary(),
            file: "rover.sysml".into(),
            before: initial.into(),
            after: updated.clone(),
        });
        assert_eq!(history.undo().unwrap().before, initial);
        assert_eq!(history.redo().unwrap().after, updated);
    }

    #[test]
    fn supports_action_flow_views_guards_and_quoted_names() {
        let initial = "package 'System Behavior' {\n    action 'Visitor Story' {\n        action 'Step One';\n    }\n}\n";
        let add_decide = SyssurCommand::CreateControlNode {
            file: "b.sysml".into(),
            owner_action: "Visitor Story".into(),
            kind: "decide".into(),
            name: "decide1".into(),
        };
        let s1 = apply_text_edit(initial, &plan_text_edit(&add_decide, initial).unwrap());
        assert!(s1.contains("decide decide1;"));

        let add_succ = SyssurCommand::CreateSuccession {
            file: "b.sysml".into(),
            owner_action: "Visitor Story".into(),
            source: "decide1".into(),
            target: "Step One".into(),
            guard: Some("timeElapsed == false".into()),
        };
        let s2 = apply_text_edit(&s1, &plan_text_edit(&add_succ, &s1).unwrap());
        assert!(s2.contains("first decide1 if timeElapsed == false then 'Step One';"));

        let upd_guard = SyssurCommand::UpdateSuccessionGuard {
            file: "b.sysml".into(),
            source: "decide1".into(),
            target: "Step One".into(),
            new_guard: "time <= 5".into(),
        };
        let s3 = apply_text_edit(&s2, &plan_text_edit(&upd_guard, &s2).unwrap());
        assert!(s3.contains("first decide1 if time <= 5 then 'Step One';"));

        let add_view = SyssurCommand::CreateView {
            file: "b.sysml".into(),
            package: "System Behavior".into(),
            name: "Visitor Arrival".into(),
            view_kind: "ActionFlowView".into(),
            expose: vec!["'System Behavior'::**".into()],
        };
        let s4 = apply_text_edit(&s3, &plan_text_edit(&add_view, &s3).unwrap());
        assert!(s4.contains("view 'Visitor Arrival' : ActionFlowView"));

        let rename = SyssurCommand::RenameElement {
            file: "b.sysml".into(),
            old_name: "Step One".into(),
            new_name: "Walk up to door".into(),
        };
        let s5 = apply_text_edit(&s4, &plan_text_edit(&rename, &s4).unwrap());
        assert!(s5.contains("'Walk up to door'"));
        assert!(!s5.contains("'Step One'"));
    }

    #[test]
    fn expands_semicolon_part_when_updating_doc_or_inserting_child() {
        let initial = "package 'System Structure' {\n    part 'Doorbell System' {\n        part Chime;\n    }\n}\n";
        let doc_cmd = SyssurCommand::UpdateElementDoc {
            file: "s.sysml".into(),
            element_name: "Chime".into(),
            new_doc: "Produces chime tones".into(),
        };
        let s1 = apply_text_edit(initial, &plan_text_edit(&doc_cmd, initial).unwrap());
        assert!(s1.contains("part Chime {\n"));
        assert!(s1.contains("doc /* Produces chime tones */"));
    }

    #[test]
    fn deletes_nested_package_and_elements_and_transforms_clause7_constructs() {
        let initial = r#"package PkgA {
    part def Rover {
        port pwr : PowerPort;
    }
    part rover1 : Rover;
}

package PkgB {
    state def Modes {
        state Off;
        state On;
    }
}
"#;
        // 1. Delete whole PkgA cleanly
        let del_pkg = SyssurCommand::DeleteElement {
            file: "m.sysml".into(),
            kind_hint: Some("package".into()),
            name: "PkgA".into(),
        };
        let after_del = apply_text_edit(initial, &plan_text_edit(&del_pkg, initial).unwrap());
        assert!(!after_del.contains("PkgA"));
        assert!(after_del.contains("package PkgB"));

        // 2. Toggle modifier `abstract` and Def<->Usage on `Rover`
        let mod_cmd = SyssurCommand::ToggleModifier {
            file: "m.sysml".into(),
            element_name: "Rover".into(),
            modifier: "abstract".into(),
        };
        let s1 = apply_text_edit(initial, &plan_text_edit(&mod_cmd, initial).unwrap());
        assert!(s1.contains("abstract part def Rover"));

        let conj_cmd = SyssurCommand::TogglePortConjugation {
            file: "m.sysml".into(),
            port_name: "pwr".into(),
        };
        let s2 = apply_text_edit(&s1, &plan_text_edit(&conj_cmd, &s1).unwrap());
        assert!(s2.contains("port pwr : ~PowerPort;"));

        let def_use = SyssurCommand::ToggleDefUsage {
            file: "m.sysml".into(),
            element_name: "Modes".into(),
        };
        let s3 = apply_text_edit(&s2, &plan_text_edit(&def_use, &s2).unwrap());
        assert!(s3.contains("state Modes {"));

        let kind_cmd = SyssurCommand::ChangeElementKind {
            file: "m.sysml".into(),
            element_name: "rover1".into(),
            new_keyword: "item".into(),
        };
        let s4 = apply_text_edit(&s3, &plan_text_edit(&kind_cmd, &s3).unwrap());
        assert!(s4.contains("item rover1 : Rover;"));

        let mv_cmd = SyssurCommand::MoveElementToPackage {
            file: "m.sysml".into(),
            element_name: "rover1".into(),
            target_package: "PkgB".into(),
        };
        let s5 = apply_text_edit(&s4, &plan_text_edit(&mv_cmd, &s4).unwrap());
        let b_idx = s5.find("package PkgB").unwrap();
        assert!(s5[b_idx..].contains("item rover1 : Rover;"));
    }

    #[test]
    fn creates_updates_and_deletes_block_attributes_and_specializes() {
        let initial = r#"package P {
    part def BaseVehicle {
        attribute weight : Real = 100.0;
    }
    part def Rover {
        attribute mass : Real = 25.0;
    }
    part unitA : Rover;
}
"#;
        // 1. Add attribute to Rover
        let add_cmd = SyssurCommand::CreateAttribute {
            file: "p.sysml".into(),
            owner_name: "Rover".into(),
            name: "maxSpeed".into(),
            type_name: Some("Real".into()),
            value_expr: Some("5.5".into()),
        };
        let s1 = apply_text_edit(initial, &plan_text_edit(&add_cmd, initial).unwrap());
        assert!(s1.contains("attribute maxSpeed : Real = 5.5;"));

        // 2. Update owned attribute on Rover
        let upd_cmd = SyssurCommand::UpdateAttribute {
            file: "p.sysml".into(),
            owner_name: "Rover".into(),
            old_name: "mass".into(),
            new_name: "dryMass".into(),
            type_name: Some("Real".into()),
            value_expr: Some("30.0".into()),
        };
        let s2 = apply_text_edit(&s1, &plan_text_edit(&upd_cmd, &s1).unwrap());
        assert!(s2.contains("attribute dryMass : Real = 30.0;"));
        assert!(!s2.contains("attribute mass"));

        // 3. Override inherited attribute locally on semicolon part usage `unitA`
        let redef_cmd = SyssurCommand::UpdateAttribute {
            file: "p.sysml".into(),
            owner_name: "unitA".into(),
            old_name: "maxSpeed".into(),
            new_name: "maxSpeed".into(),
            type_name: Some("Real".into()),
            value_expr: Some("8.0".into()),
        };
        let s3 = apply_text_edit(&s2, &plan_text_edit(&redef_cmd, &s2).unwrap());
        assert!(s3.contains("part unitA : Rover {"));
        assert!(s3.contains("attribute redefines maxSpeed : Real = 8.0;"));

        // 4. Specialize BaseVehicle from Rover
        let spec_cmd = SyssurCommand::UpdatePartDefSpecializes {
            file: "p.sysml".into(),
            def_name: "Rover".into(),
            specializes: "BaseVehicle".into(),
        };
        let s4 = apply_text_edit(&s3, &plan_text_edit(&spec_cmd, &s3).unwrap());
        assert!(s4.contains("part def Rover :> BaseVehicle {"));

        // 5. Delete attribute from Rover
        let del_cmd = SyssurCommand::DeleteAttribute {
            file: "p.sysml".into(),
            owner_name: "Rover".into(),
            attr_name: "dryMass".into(),
        };
        let s5 = apply_text_edit(&s4, &plan_text_edit(&del_cmd, &s4).unwrap());
        assert!(!s5.contains("dryMass"));
        assert!(s5.contains("attribute maxSpeed : Real = 5.5;"));
    }
}
