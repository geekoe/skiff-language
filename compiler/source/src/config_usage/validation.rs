use std::collections::BTreeMap;

use crate::shared::ast::{Expr, Literal, TypeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigIntrinsic {
    Require,
    Optional,
    Has,
    Get,
}

impl ConfigIntrinsic {
    pub(super) fn call_name(self) -> &'static str {
        match self {
            ConfigIntrinsic::Require => "config.require",
            ConfigIntrinsic::Optional => "config.optional",
            ConfigIntrinsic::Has => "config.has",
            ConfigIntrinsic::Get => "config.get",
        }
    }
}

pub(super) fn push_indirect_config_intrinsic_violation(path: &str, violations: &mut Vec<String>) {
    violations.push(format!(
        "{path}: config require/optional/has cannot be aliased or called indirectly; use direct config.require<T>(const-path), config.optional<T>(const-path), or config.has(const-path)"
    ));
}

pub(super) fn push_legacy_values_violation(path: &str, violations: &mut Vec<String>) {
    violations.push(format!(
        "{path}: values.* has been removed; use config.require<T>(path) or config.optional<T>(path)"
    ));
}

pub(super) fn validate_config_path(path: &str, context: &str) -> Option<String> {
    for segment in path.split('.') {
        if segment.is_empty() {
            return Some(format!(
                "{context} path \"{path}\" has an empty segment; expected each dotted segment to match [A-Za-z_][A-Za-z0-9_-]*"
            ));
        }
        if !is_valid_config_path_segment(segment) {
            return Some(format!(
                "{context} path \"{path}\" has invalid segment \"{segment}\"; expected each dotted segment to match [A-Za-z_][A-Za-z0-9_-]*"
            ));
        }
    }
    None
}

fn is_valid_config_path_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

pub(super) fn const_string_expr(
    expr: &Expr,
    const_strings: &BTreeMap<String, String>,
) -> Option<String> {
    match expr {
        Expr::Literal(Literal::String(value)) => Some(value.clone()),
        Expr::Identifier(name) => const_strings.get(name).cloned(),
        Expr::Binary {
            op: crate::shared::ast::BinaryOp::Add,
            left,
            right,
        } => {
            let mut value = const_string_expr(left, const_strings)?;
            value.push_str(&const_string_expr(right, const_strings)?);
            Some(value)
        }
        _ => None,
    }
}

pub(super) fn config_intrinsic_callee(callee: &Expr) -> Option<(ConfigIntrinsic, &[TypeRef])> {
    match callee {
        Expr::Field { object, field } if matches!(object.as_ref(), Expr::Identifier(root) if root == "config") => {
            config_intrinsic_from_field(field).map(|intrinsic| (intrinsic, &[][..]))
        }
        Expr::Generic { callee, type_args } => {
            config_intrinsic_field(callee).map(|intrinsic| (intrinsic, type_args.as_slice()))
        }
        _ => None,
    }
}

pub(super) fn config_intrinsic_field(expr: &Expr) -> Option<ConfigIntrinsic> {
    match expr {
        Expr::Field { object, field } if matches!(object.as_ref(), Expr::Identifier(root) if root == "config") => {
            config_intrinsic_from_field(field)
        }
        _ => None,
    }
}

fn config_intrinsic_from_field(field: &str) -> Option<ConfigIntrinsic> {
    match field {
        "require" => Some(ConfigIntrinsic::Require),
        "optional" => Some(ConfigIntrinsic::Optional),
        "has" => Some(ConfigIntrinsic::Has),
        "get" => Some(ConfigIntrinsic::Get),
        _ => None,
    }
}
